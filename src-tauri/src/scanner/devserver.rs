//! Where the site is actually running.
//!
//! Ordered by how reliable each source is. High confidence gets used
//! automatically; anything less is offered as a list, and you can always type a
//! URL over the top of whatever this decides.

use regex::Regex;
use yaml_rust2::{Yaml, YamlLoader};

use super::jsobj;
use super::log::ScanLog;
use super::types::{DetectorOutput, DevServerDiscovery};
use super::walk::{FileIndex, ReadBudget};

/// Confidence at or above this is used without asking.
pub const AUTO_USE: f64 = 0.85;

pub fn run(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> DetectorOutput {
    let mut out = DetectorOutput::default();
    // Cheap, and this runs last, so once the deadline has passed there is
    // nothing to be gained by reading more files.
    if index.out_of_time() {
        log.note("scan timeout reached, so dev server detection was skipped");
        return out;
    }
    let mut push = |candidate: DevServerDiscovery, log: &mut ScanLog| {
        log.candidate(&candidate.url, &candidate.source, candidate.confidence);
        out.dev_servers.push(candidate);
    };

    // Lando
    for rel in index.by_name(".lando.yml") {
        let file = rel.to_string_lossy().to_string();
        let Some(text) = budget.read(index, rel, log) else { continue };
        let Ok(docs) = YamlLoader::load_from_str(&text) else {
            log.parse_failure(&file, None, "", "yaml", "could not parse .lando.yml");
            continue;
        };
        let Some(doc) = docs.first() else { continue };

        let name = doc["name"].as_str().map(str::to_string);

        // The name-derived host is the site itself, and Lando always serves it.
        if let Some(name) = &name {
            push(
                DevServerDiscovery {
                    url: format!("https://{name}.lndo.site"),
                    port: None,
                    source: ".lando.yml name".into(),
                    confidence: 0.9,
                    responding: None,
                },
                log,
            );
        } else {
            log.discarded(&file, "<name>", "no name key, so no site hostname to derive");
        }

        // Proxy entries are a mixed bag. One of them is usually the site, and
        // the rest are tooling: a Vite HMR server, Mailhog, phpMyAdmin. A host
        // that starts with the project name is the site; the others are not,
        // and offering a Vite port as the site URL is worse than useless.
        if let Yaml::Hash(proxy) = &doc["proxy"] {
            for (_service, hosts) in proxy.iter() {
                let Yaml::Array(hosts) = hosts else { continue };
                for host in hosts {
                    let Some(host) = host.as_str() else { continue };
                    let host = host.split('/').next().unwrap_or(host);
                    let is_site = name
                        .as_ref()
                        .map(|n| host.starts_with(&format!("{n}.")))
                        .unwrap_or(false);
                    if !is_site {
                        log.discarded(
                            &file,
                            host,
                            "proxy host does not start with the project name, so it is tooling rather than the site",
                        );
                    }
                    push(
                        DevServerDiscovery {
                            url: format!("https://{host}"),
                            port: None,
                            source: ".lando.yml proxy".into(),
                            confidence: if is_site { 0.95 } else { 0.45 },
                            responding: None,
                        },
                        log,
                    );
                }
            }
        }
    }

    // DDEV
    for rel in index.files.iter().filter(|p| p.ends_with(".ddev/config.yaml")) {
        let Some(text) = budget.read(index, rel, log) else { continue };
        if let Ok(docs) = YamlLoader::load_from_str(&text) {
            if let Some(name) = docs.first().and_then(|d| d["name"].as_str()) {
                push(
                    DevServerDiscovery {
                        url: format!("https://{name}.ddev.site"),
                        port: None,
                        source: ".ddev/config.yaml".into(),
                        confidence: 0.9,
                        responding: None,
                    },
                    log,
                );
            }
        }
    }

    // Vite
    for rel in index.by_stem("vite.config") {
        let file = rel.to_string_lossy().to_string();
        let Some(text) = budget.read(index, rel, log) else { continue };
        match vite_origin(&text) {
            Some((origin, port)) => push(
                DevServerDiscovery {
                    url: origin,
                    port,
                    source: file,
                    confidence: 0.9,
                    responding: None,
                },
                log,
            ),
            None => log.note(format!("{file} has no server.port, so Vite's default applies")),
        }
    }

    // Docker Compose
    for name in ["docker-compose.yml", "docker-compose.yaml", "compose.yml"] {
        for rel in index.by_name(name) {
            let file = rel.to_string_lossy().to_string();
            let Some(text) = budget.read(index, rel, log) else { continue };
            for port in compose_web_ports(&text, &file, log) {
                push(
                    DevServerDiscovery {
                        url: format!("http://localhost:{port}"),
                        port: Some(port),
                        source: file.clone(),
                        confidence: 0.6,
                        responding: None,
                    },
                    log,
                );
            }
        }
    }

    // package.json: an explicit --port in a dev script, then the framework's own default.
    for rel in index.by_name("package.json") {
        if rel.components().count() > 1 {
            continue;
        }
        let Some(text) = budget.read(index, rel, log) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };

        let port_re = Regex::new(r"(?:--port[= ]|(?:^|\s)-p )(\d{2,5})").unwrap();
        for script in ["dev", "start", "serve"] {
            if let Some(command) = json["scripts"][script].as_str() {
                if let Some(caps) = port_re.captures(command) {
                    if let Ok(port) = caps[1].parse::<u16>() {
                        push(
                            DevServerDiscovery {
                                url: format!("http://localhost:{port}"),
                                port: Some(port),
                                source: format!("package.json scripts.{script}"),
                                confidence: 0.7,
                                responding: None,
                            },
                            log,
                        );
                    }
                }
            }
        }

        for (package, port) in FRAMEWORK_DEFAULTS {
            let present = ["dependencies", "devDependencies"]
                .iter()
                .any(|section| json[section].get(package).is_some());
            if present {
                push(
                    DevServerDiscovery {
                        url: format!("http://localhost:{port}"),
                        port: Some(*port),
                        source: format!("{package} default port"),
                        confidence: 0.4,
                        responding: None,
                    },
                    log,
                );
            }
        }
    }

    demote_bundlers(&mut out.dev_servers, log);
    dedupe(&mut out.dev_servers);
    if out.dev_servers.is_empty() {
        log.note("no dev server config found in Lando, DDEV, Vite, Compose or package.json");
    }
    out
}

/// A bundler's dev server is not the site when there is a whole environment
/// running the site.
///
/// In a Drupal project under Lando, Vite serves assets on 5173 and PHP serves
/// the pages. Offering localhost:5173 as the URL to test loads a module graph,
/// not a page, which is a confusing way to start.
fn demote_bundlers(candidates: &mut [DevServerDiscovery], log: &mut ScanLog) {
    let has_environment = candidates
        .iter()
        .any(|c| c.source.contains("lando") || c.source.contains("ddev"));
    if !has_environment {
        return;
    }
    for candidate in candidates.iter_mut() {
        let bundler = candidate.source.starts_with("vite.config")
            || candidate.source.contains("/vite.config")
            || candidate.source.contains("default port")
            || candidate.source.starts_with("package.json");
        if bundler && candidate.confidence > 0.5 {
            log.note(format!(
                "{} demoted: Lando or DDEV is serving the site, so this is the asset server",
                candidate.url
            ));
            candidate.confidence = 0.5;
        }
    }
}

const FRAMEWORK_DEFAULTS: &[(&str, u16)] = &[
    ("vite", 5173),
    ("@sveltejs/kit", 5173),
    ("next", 3000),
    ("nuxt", 3000),
    ("react-scripts", 3000),
    ("astro", 4321),
    ("gatsby", 8000),
    ("@angular/cli", 4200),
];

/// `server: { port, host, https }` read statically out of a Vite config.
fn vite_origin(text: &str) -> Option<(String, Option<u16>)> {
    let blanked: Vec<char> = jsobj::blank_comments(text).chars().collect();
    let colon = jsobj::find_key(&blanked, "server", 0)?;
    let open = jsobj::object_after_colon(&blanked, colon)?;
    let entries = jsobj::entries(&blanked, open);
    let get = |key: &str| {
        entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.value.trim_matches(['"', '\'']).to_string())
    };
    let port: Option<u16> = get("port").and_then(|p| p.parse().ok());
    let https = get("https").map(|v| v != "false").unwrap_or(false);
    let host = get("host").unwrap_or_default();
    // `host: true` means "listen on every interface", not "the hostname is true".
    let host = if host.is_empty() || host == "true" || host == "false" {
        "localhost".to_string()
    } else {
        host
    };
    let scheme = if https { "https" } else { "http" };
    let port = port.or(Some(5173));
    Some((format!("{scheme}://{host}:{}", port.unwrap()), port))
}

/// Published host ports on services that look like a web server.
fn compose_web_ports(text: &str, file: &str, log: &mut ScanLog) -> Vec<u16> {
    let Ok(docs) = YamlLoader::load_from_str(text) else {
        log.parse_failure(file, None, "", "yaml", "could not parse compose file");
        return Vec::new();
    };
    let Some(doc) = docs.first() else { return Vec::new() };
    let Yaml::Hash(services) = &doc["services"] else { return Vec::new() };

    let mut out = Vec::new();
    for (name, service) in services.iter() {
        let name = name.as_str().unwrap_or_default();
        let webish = ["web", "nginx", "apache", "app", "php", "frontend", "node", "httpd"]
            .iter()
            .any(|m| name.contains(m));
        if !webish {
            log.discarded(file, name, "service name does not look like a web server");
            continue;
        }
        if let Yaml::Array(ports) = &service["ports"] {
            for mapping in ports {
                let Some(mapping) = mapping.as_str() else { continue };
                // "8080:80" and "127.0.0.1:8080:80" both publish 8080.
                let parts: Vec<&str> = mapping.split(':').collect();
                let host_port = match parts.len() {
                    2 => parts[0],
                    3 => parts[1],
                    _ => continue,
                };
                if let Ok(port) = host_port.parse::<u16>() {
                    out.push(port);
                }
            }
        }
    }
    out
}

/// Keep the highest-confidence entry per URL, in confidence order.
fn dedupe(candidates: &mut Vec<DevServerDiscovery>) {
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap()
            .then(a.url.cmp(&b.url))
    });
    let mut seen = std::collections::BTreeSet::new();
    candidates.retain(|c| seen.insert(c.url.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vite_config_with_a_port_beats_the_default() {
        let found = vite_origin("export default { server: { port: 3001 } }").unwrap();
        assert_eq!(found.0, "http://localhost:3001");
        assert_eq!(found.1, Some(3001));
    }

    #[test]
    fn host_true_means_every_interface_not_a_hostname() {
        let found = vite_origin("export default { server: { host: true, port: 5174 } }").unwrap();
        assert_eq!(found.0, "http://localhost:5174");
    }

    #[test]
    fn compose_ports_come_from_web_services_in_either_mapping_form() {
        let mut log = ScanLog::new("test");
        let yaml = "services:\n  web:\n    ports:\n      - \"8080:80\"\n  db:\n    ports:\n      - \"3306:3306\"\n  nginx:\n    ports:\n      - \"127.0.0.1:8081:80\"\n";
        let mut found = compose_web_ports(yaml, "docker-compose.yml", &mut log);
        found.sort();
        assert_eq!(found, vec![8080, 8081]);
    }

    #[test]
    fn a_bundler_port_never_outranks_the_environment_serving_the_site() {
        let mut log = ScanLog::new("test");
        let mut candidates = vec![
            DevServerDiscovery { url: "http://localhost:5173".into(), port: Some(5173), source: "web/themes/x/vite.config.js".into(), confidence: 0.9, responding: None },
            DevServerDiscovery { url: "https://x.lndo.site".into(), port: None, source: ".lando.yml name".into(), confidence: 0.9, responding: None },
        ];
        demote_bundlers(&mut candidates, &mut log);
        dedupe(&mut candidates);
        assert_eq!(candidates[0].url, "https://x.lndo.site");
    }

    #[test]
    fn a_vite_only_project_keeps_its_bundler_url() {
        let mut log = ScanLog::new("test");
        let mut candidates = vec![DevServerDiscovery {
            url: "http://localhost:5173".into(),
            port: Some(5173),
            source: "vite.config.js".into(),
            confidence: 0.9,
            responding: None,
        }];
        demote_bundlers(&mut candidates, &mut log);
        assert_eq!(candidates[0].confidence, 0.9);
    }

    #[test]
    fn a_lando_proxy_for_tooling_does_not_outrank_the_site_itself() {
        // Reproduces bucknell-be-the-ray: the only proxy entry is Vite's HMR
        // server, and the site lives at the name-derived host.
        let mut candidates = vec![
            DevServerDiscovery { url: "https://vite-x.lndo.site:5173".into(), port: None, source: ".lando.yml proxy".into(), confidence: 0.45, responding: None },
            DevServerDiscovery { url: "https://x.lndo.site".into(), port: None, source: ".lando.yml name".into(), confidence: 0.9, responding: None },
        ];
        dedupe(&mut candidates);
        assert_eq!(candidates[0].url, "https://x.lndo.site");
    }

    #[test]
    fn the_higher_confidence_source_survives_deduping() {
        let mut candidates = vec![
            DevServerDiscovery { url: "http://localhost:5173".into(), port: None, source: "default".into(), confidence: 0.4, responding: None },
            DevServerDiscovery { url: "http://localhost:5173".into(), port: None, source: "vite.config.js".into(), confidence: 0.9, responding: None },
        ];
        dedupe(&mut candidates);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, "vite.config.js");
    }
}
