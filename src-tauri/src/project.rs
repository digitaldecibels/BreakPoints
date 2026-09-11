//! Opening a project: scan it, decide what wins, and hand the chrome something
//! it can draw the scan sheet from.
//!
//! The precedence rule is the important part. If `breakpoints.md` exists it
//! wins, full stop. The scan still runs, because that is what makes change
//! detection work, but it never touches the file without asking.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::model::{AppConfig, PanelState, UrlStatus, Viewport};
use crate::scanner::types::ScanReport;
use crate::state::{ProjectContext, Shared};
use crate::{canvas, config, generate, project_file, scanner, util, watcher};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpened {
    pub root: String,
    pub name: String,
    /// True when a project file is driving the viewports.
    pub project_driven: bool,
    pub report: ScanReport,
    pub recommendation: generate::Recommendation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_file: Option<project_file::ProjectFile>,
    /// Set when a project file exists but will not parse. The old config is
    /// kept and the toolbar shows a warning chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_file_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_url: Option<String>,
    pub url_status: UrlStatus,
    /// True when the config on disk no longer matches what the code says.
    pub source_changed: bool,
}

/// Scan a folder and work out what should happen, without changing anything.
pub async fn open(app: &AppHandle, state: &Shared, root: PathBuf) -> Result<ProjectOpened, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }

    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    // Reopening a project should be instant.
    let report = match scanner::cached(&root) {
        Some(report) => {
            let _ = app.emit("scan:cached", serde_json::json!({ "root": root.to_string_lossy() }));
            report
        }
        None => scan_now(app, &root).await?,
    };

    let (strategy, fixed, edge_testing) = {
        let config = state.config.lock().unwrap();
        (config.height_strategy, config.fixed_height, config.edge_testing)
    };
    let mut recommendation =
        generate::recommend(&report.breakpoints, strategy, fixed, edge_testing);

    // Static confidence gets us a shortlist; only asking settles it. A project
    // can carry a Lando file and a DDEV file and only one of them is running.
    let dev_url = pick_dev_url(&report).await;
    recommendation.dev_url = dev_url.clone();

    let (project, project_error) = match project_file::read(&root) {
        Some(Ok(file)) => (Some(file), None),
        Some(Err(err)) => (None, Some(err)),
        None => (None, None),
    };

    let source_changed = project
        .as_ref()
        .and_then(|f| f.source_hash.clone())
        .map(|hash| hash != report.source_hash)
        .unwrap_or(false);

    let url = project
        .as_ref()
        .and_then(|f| f.url.clone())
        .or_else(|| dev_url.clone());

    let url_status = match &url {
        // The dev URL was already probed while it was being chosen.
        Some(chosen) if Some(chosen) == dev_url.as_ref() => UrlStatus::Ok,
        Some(url) => probe(url).await,
        None => UrlStatus::Unknown,
    };

    *state.project.lock().unwrap() = Some(ProjectContext {
        root: root.clone(),
        name: name.clone(),
        project_driven: project.is_some(),
        dev_url: dev_url.clone(),
        report: Some(report.clone()),
    });
    *state.url_status.lock().unwrap() = url_status.clone();

    Ok(ProjectOpened {
        root: root.to_string_lossy().to_string(),
        name,
        project_driven: project.is_some(),
        report,
        recommendation,
        project_file: project,
        project_file_error: project_error,
        dev_url,
        url_status,
        source_changed,
    })
}

/// Run the detectors, streaming each row to the chrome as it lands, and write
/// the diagnostics log.
pub async fn scan_now(app: &AppHandle, root: &Path) -> Result<ScanReport, String> {
    let handle = app.clone();
    let root_owned = root.to_path_buf();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let emitter = handle.clone();
        scanner::scan(&root_owned, move |row| {
            let _ = emitter.emit("scan:row", row);
        })
    })
    .await
    .map_err(|e| format!("scan did not finish: {e}"))?;

    let mut report = outcome.report;
    if let Ok(dir) = config::scan_log_dir(app) {
        let slug = util::slugify(&report.project_name);
        if let Some(path) = outcome.log.write(&dir, &slug) {
            report.log_path = Some(path.to_string_lossy().to_string());
        }
    }
    scanner::remember(root, &report);
    Ok(report)
}

/// Ask the top candidates which one is actually serving the site.
///
/// A project that has lived through two local environments carries both their
/// config files, and neither one says which is running today. Rather than
/// guessing from file mtimes, Break/Points knocks on each door in confidence
/// order and takes the first that answers. If none do, the best-scoring
/// candidate is still offered, labelled honestly.
async fn pick_dev_url(report: &ScanReport) -> Option<String> {
    let candidates: Vec<(String, f64)> = report
        .dev_servers
        .iter()
        .map(|d| (d.url.clone(), d.confidence))
        .collect();
    choose_dev_url(&candidates, |url| async move { probe(&url).await }).await
}

/// The candidate to use, given a way to ask each one whether it is there.
///
/// The prober is a parameter so this can be tested without a network. The
/// ordering and the fallback are the parts that were wrong, not the request.
pub async fn choose_dev_url<F, Fut>(
    candidates: &[(String, f64)],
    mut ask: F,
) -> Option<String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = UrlStatus>,
{
    let shortlist: Vec<&(String, f64)> = candidates
        .iter()
        .filter(|(_, confidence)| *confidence >= scanner::devserver::AUTO_USE)
        .take(4)
        .collect();

    for (url, _) in &shortlist {
        if ask(url.clone()).await == UrlStatus::Ok {
            return Some(url.clone());
        }
    }
    // Nothing answered. Offer the best guess anyway, labelled honestly, so the
    // panels show a failure at the right hostname rather than nothing at all.
    shortlist
        .first()
        .map(|(url, _)| url.clone())
        .or_else(|| candidates.first().map(|(url, _)| url.clone()))
}

/// Is this a URL worth remembering, as opposed to an empty canvas?
pub fn is_real_url(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty() && !url.starts_with("about:")
}

/// What a status code means for "is the site there?".
///
/// Split out of `probe` so the rule can be tested. 401 and 403 mean the site
/// is there and asking who you are, which is ordinary on a staging box. 404
/// and 5xx at the root mean it is not, and treating those as success is what
/// sent a Lando project to a dead DDEV hostname.
pub fn classify(status: u16) -> UrlStatus {
    let responding = (200..400).contains(&status) || status == 401 || status == 403;
    if responding {
        UrlStatus::Ok
    } else {
        UrlStatus::NotResponding
    }
}

/// Is the site actually there? A dev URL that is not answering is still
/// offered, it is just labelled honestly.
///
/// "Something answered" is not the same question, and asking it that way got
/// the wrong answer on a real machine. Lando and DDEV both point their
/// hostnames at 127.0.0.1, and Lando's proxy answers for *every* `.ddev.site`
/// name with a 404. So a project with a leftover `.ddev/config.yaml` and a
/// running Lando container was sent to the DDEV hostname, every panel loaded
/// the proxy's 404 page, and the live site sitting one candidate down the list
/// never got a turn.
pub async fn probe(url: &str) -> UrlStatus {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        // Lando and DDEV both serve behind their own certificate authority.
        // Whether the certificate is trusted is not the question here.
        .danger_accept_invalid_certs(true)
        .build()
    else {
        return UrlStatus::Unknown;
    };

    // A HEAD is cheaper, but plenty of dev servers answer it badly, so a
    // refusal is worth one retry as a GET before believing it.
    let status = match client.head(url).send().await {
        Ok(response) => Some(response.status()),
        Err(_) => client.get(url).send().await.ok().map(|r| r.status()),
    };
    match status {
        Some(status) => classify(status.as_u16()),
        None => UrlStatus::NotResponding,
    }
}

/// Commit a viewport set: spawn the panels, remember the project, and start
/// watching. Writing `breakpoints.md` is a separate, explicit step.
pub async fn apply(
    app: &AppHandle,
    state: &Shared,
    viewports: Vec<Viewport>,
    url: String,
) -> Result<f64, String> {
    let total = canvas::spawn(app, state, viewports.clone(), &url).await?;

    let root = state.project.lock().unwrap().as_ref().map(|p| p.root.clone());

    {
        let mut config = state.config.lock().unwrap();
        config.last_url = Some(url.clone());
        if let Some(root) = &root {
            let key = root.to_string_lossy().to_string();
            config.last_project = Some(key.clone());
            let record = config.projects.entry(key.clone()).or_default();
            record.last_scan = Some(util::now_iso());
            // A project opened with no URL of its own gets a blank canvas, and
            // remembering "about:blank belongs to this project" is worse than
            // useless: the association is what later decides whether a URL is
            // safe to write into a committed breakpoints.md.
            if is_real_url(&url) {
                record.url = Some(url.clone());
                config.url_to_project.insert(url.clone(), key);
            }
        } else {
            let active = config.active_profile.clone();
            if let Some(profile) = config.profiles.get_mut(&active).filter(|p| !p.locked) {
                profile.viewports = viewports.clone();
            }
        }
        save_locked(app, &config);
    }

    if let Some(root) = root {
        match watcher::start(app.clone(), &root) {
            Ok(handle) => *state.watcher.lock().unwrap() = Some(handle),
            Err(err) => eprintln!("[breakpoints] could not watch {}: {err}", root.display()),
        }
    }

    // Tell the toolbar whether the URL is answering, without blocking the
    // panels on it.
    let status = probe(&url).await;
    *state.url_status.lock().unwrap() = status.clone();
    if status == UrlStatus::NotResponding {
        canvas::set_all_panel_states(app, state, PanelState::Failed(0));
    }
    let _ = app.emit("url:status", &status);

    Ok(total)
}

/// Write `breakpoints.md` into the project root.
///
/// Always asked for explicitly, never on a timer: this lands in a repo that
/// might be a client's, and it is meant to be committed.
pub fn write_file(
    app: &AppHandle,
    state: &Shared,
    viewports: &[Viewport],
    url: Option<String>,
) -> Result<String, String> {
    let Some(project) = state.project.lock().unwrap().clone() else {
        return Err("no project is open, so there is nowhere to write it".into());
    };

    let (source, hash) = project
        .report
        .as_ref()
        .map(|r| {
            (
                r.frameworks
                    .first()
                    .map(|f| f.source_file.clone())
                    .unwrap_or_else(|| "css media queries".into()),
                r.source_hash.clone(),
            )
        })
        .unwrap_or_default();

    let (zoom, sync) = {
        let canvas = state.canvas.lock().unwrap();
        (canvas.zoom_to_fit, canvas.sync_on)
    };

    let file = project_file::ProjectFile {
        viewports: viewports.to_vec(),
        url,
        zoom_to_fit: Some(zoom),
        scroll_sync: Some(sync),
        source: Some(source),
        source_hash: Some(hash.clone()),
        generated: Some(util::today_iso()),
    };

    let path = project_file::write(&project.root, &file)?;

    {
        let mut config = state.config.lock().unwrap();
        let key = project.root.to_string_lossy().to_string();
        let record = config.projects.entry(key).or_default();
        record.source_hash = Some(hash);
        save_locked(app, &config);
    }
    if let Some(context) = state.project.lock().unwrap().as_mut() {
        context.project_driven = true;
    }

    Ok(path.to_string_lossy().to_string())
}

/// What to load at startup, in the order section 13 lays out.
pub async fn restore(app: &AppHandle, state: &Shared) -> Option<ProjectOpened> {
    let (last_project, last_url) = {
        let config = state.config.lock().unwrap();
        (config.last_project.clone(), config.last_url.clone())
    };

    let root = last_project
        .or_else(|| {
            let config = state.config.lock().unwrap();
            last_url
                .as_ref()
                .and_then(|url| config.url_to_project.get(url).cloned())
        })
        .map(PathBuf::from)?;

    if !root.is_dir() {
        return None;
    }
    open(app, state, root).await.ok()
}

fn save_locked(app: &AppHandle, config: &AppConfig) {
    if let Err(err) = config::save(app, config) {
        eprintln!("[breakpoints] could not save config: {err}");
    }
}

/// The folder a URL was last associated with, so typing that URL reconnects the
/// project automatically.
pub fn project_for_url(state: &Shared, url: &str) -> Option<PathBuf> {
    let config = state.config.lock().unwrap();
    config
        .url_to_project
        .get(url)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Guard every project read: nothing above the project root is ever opened.
pub fn inside_project(root: &Path, candidate: &Path) -> bool {
    let (Ok(root), Ok(candidate)) = (root.canonicalize(), candidate.canonicalize()) else {
        return false;
    };
    candidate.starts_with(root)
}

pub fn app_project_root(app: &AppHandle) -> Option<PathBuf> {
    app.state::<Shared>()
        .project
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.root.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidate list as the scanner hands it over: url and confidence.
    fn candidates(urls: &[(&str, f64)]) -> Vec<(String, f64)> {
        urls.iter().map(|(u, c)| (u.to_string(), *c)).collect()
    }

    /// A prober that says yes to exactly the URLs named.
    fn only_these(live: &'static [&'static str]) -> impl FnMut(String) -> std::future::Ready<UrlStatus>
    {
        move |url: String| {
            std::future::ready(if live.contains(&url.as_str()) {
                UrlStatus::Ok
            } else {
                UrlStatus::NotResponding
            })
        }
    }

    #[test]
    fn an_empty_canvas_is_not_a_url_worth_remembering() {
        // The project-to-URL map is what later decides whether a URL may be
        // written into a committed breakpoints.md, so a meaningless entry in it
        // is not harmless.
        assert!(!is_real_url("about:blank"));
        assert!(!is_real_url(""));
        assert!(!is_real_url("   "));
        assert!(is_real_url("https://mine.lndo.site"));
        assert!(is_real_url("http://localhost:1420"));
    }

    #[test]
    fn a_site_that_is_there_is_there() {
        assert_eq!(classify(200), UrlStatus::Ok);
        assert_eq!(classify(204), UrlStatus::Ok);
        assert_eq!(classify(301), UrlStatus::Ok);
        assert_eq!(classify(302), UrlStatus::Ok);
    }

    #[test]
    fn a_site_asking_who_you_are_is_still_a_site() {
        // Ordinary on a staging box behind basic auth.
        assert_eq!(classify(401), UrlStatus::Ok);
        assert_eq!(classify(403), UrlStatus::Ok);
    }

    #[test]
    fn a_proxy_saying_it_has_never_heard_of_you_is_not_a_site() {
        // This is the bug. Lando's proxy answers for every .ddev.site name
        // with a 404, so "something answered" sent a Lando project to a dead
        // DDEV hostname and every panel loaded the proxy's error page.
        assert_eq!(classify(404), UrlStatus::NotResponding);
        assert_eq!(classify(500), UrlStatus::NotResponding);
        assert_eq!(classify(502), UrlStatus::NotResponding);
        assert_eq!(classify(503), UrlStatus::NotResponding);
    }

    #[tokio::test]
    async fn the_first_candidate_that_answers_wins() {
        let list = candidates(&[
            ("https://dead.ddev.site", 0.9),
            ("https://live.lndo.site", 0.9),
        ]);
        let chosen = choose_dev_url(&list, only_these(&["https://live.lndo.site"])).await;
        assert_eq!(chosen.as_deref(), Some("https://live.lndo.site"));
    }

    #[tokio::test]
    async fn a_higher_ranked_candidate_that_answers_is_not_passed_over() {
        let list = candidates(&[
            ("https://live.lndo.site", 0.9),
            ("https://other.ddev.site", 0.9),
        ]);
        let chosen = choose_dev_url(
            &list,
            only_these(&["https://live.lndo.site", "https://other.ddev.site"]),
        )
        .await;
        assert_eq!(chosen.as_deref(), Some("https://live.lndo.site"));
    }

    #[tokio::test]
    async fn when_nothing_answers_the_best_guess_is_still_offered() {
        // Better to fail at the right hostname than to show nothing: the
        // panels then say "not responding" about a URL worth starting.
        let list = candidates(&[
            ("https://first.lndo.site", 0.9),
            ("https://second.ddev.site", 0.9),
        ]);
        let chosen = choose_dev_url(&list, only_these(&[])).await;
        assert_eq!(chosen.as_deref(), Some("https://first.lndo.site"));
    }

    #[tokio::test]
    async fn a_low_confidence_guess_is_never_probed_but_can_still_be_the_fallback() {
        // Below AUTO_USE a candidate is a hint, not a claim, so it does not get
        // to win a race it was never entered in.
        let list = candidates(&[("http://localhost:5173", 0.5)]);
        let chosen = choose_dev_url(&list, |url: String| {
            panic!("probed {url}, which is below the auto-use threshold");
            #[allow(unreachable_code)]
            std::future::ready(UrlStatus::Ok)
        })
        .await;
        assert_eq!(chosen.as_deref(), Some("http://localhost:5173"));
    }

    #[tokio::test]
    async fn a_project_with_no_dev_server_gets_no_url() {
        let chosen = choose_dev_url(&[], only_these(&[])).await;
        assert_eq!(chosen, None);
    }

    #[tokio::test]
    async fn only_the_first_four_candidates_are_asked() {
        // Each probe costs up to three seconds, and a project that has lived
        // through several environments can carry a lot of stale config.
        let list = candidates(&[
            ("https://a", 0.9),
            ("https://b", 0.9),
            ("https://c", 0.9),
            ("https://d", 0.9),
            ("https://e", 0.9),
        ]);
        let chosen = choose_dev_url(&list, only_these(&["https://e"])).await;
        assert_eq!(
            chosen.as_deref(),
            Some("https://a"),
            "the fifth was never asked, so the best guess is the fallback"
        );
    }
}
