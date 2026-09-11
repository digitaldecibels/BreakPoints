//! Small shared helpers. Nothing here knows about Tauri.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Accept what a person types in a URL bar. A bare host becomes https, because
/// that is what a dev URL almost always is now; an explicit scheme is left
/// alone so `http://localhost:5173` still works.
pub fn normalize_url(input: &str) -> url::Url {
    let s = input.trim();
    if s.is_empty() {
        return url::Url::parse("about:blank").unwrap();
    }
    let full = if s.contains("://") {
        s.to_string()
    } else if s.starts_with("localhost") || s.starts_with("127.0.0.1") {
        format!("http://{s}")
    } else {
        format!("https://{s}")
    };
    url::Url::parse(&full).unwrap_or_else(|_| url::Url::parse("about:blank").unwrap())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// RFC 3339 in UTC, which is what goes in the config and in `breakpoints.md`.
pub fn now_iso() -> String {
    format_iso(now_ms() / 1000)
}

/// Just the date, for the `generated:` line in `breakpoints.md`.
pub fn today_iso() -> String {
    format_iso(now_ms() / 1000)[..10].to_string()
}

fn format_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil conversion. Avoids pulling in a date crate
/// for the two timestamps the app writes.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Eight hex characters. Long enough to notice a change, short enough to read
/// in a Markdown comment.
pub fn short_hash(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\x1f");
    }
    let digest = hasher.finalize();
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// A URL-safe random string, used for the callback nonce and the bridge token.
pub fn random_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// A folder name turned into something safe for a log filename.
pub fn slugify(input: &str) -> String {
    let s: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !last_dash {
                out.push(c);
            }
            last_dash = true;
        } else {
            out.push(c);
            last_dash = false;
        }
    }
    if out.is_empty() {
        "project".into()
    } else {
        out
    }
}

/// Shared test helpers. Only compiled for tests.
#[cfg(test)]
pub mod tests_support {
    use std::io::Write;
    use std::process::Command;

    /// Check that a string of JavaScript parses, using node.
    ///
    /// Two of this app's most important pieces of code are JavaScript held in
    /// Rust strings: the script injected into every panel, and the audit
    /// probes. Nothing compiles either of them, so a stray brace ships and then
    /// fails silently inside someone else's page.
    ///
    /// Node is already required to build this app. If it is somehow missing the
    /// check is skipped rather than failed, because a missing toolchain is not
    /// a broken script.
    pub fn assert_js_parses(source: &str, what: &str) {
        let Ok(dir) = tempdir() else {
            eprintln!("skipping the {what} syntax check: no writable temp dir");
            return;
        };
        let file = dir.join(format!("{}.js", what.replace(' ', "-")));
        let Ok(mut handle) = std::fs::File::create(&file) else {
            eprintln!("skipping the {what} syntax check: could not write {file:?}");
            return;
        };
        if handle.write_all(source.as_bytes()).is_err() {
            eprintln!("skipping the {what} syntax check: could not write {file:?}");
            return;
        }
        drop(handle);

        let output = match Command::new("node").arg("--check").arg(&file).output() {
            Ok(output) => output,
            Err(_) => {
                eprintln!("skipping the {what} syntax check: node is not on PATH");
                return;
            }
        };
        let _ = std::fs::remove_file(&file);
        assert!(
            output.status.success(),
            "{what} is not valid JavaScript:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn tempdir() -> std::io::Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join("breakpoints-js-check");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hosts_become_https_and_localhost_stays_http() {
        assert_eq!(normalize_url("example.com").as_str(), "https://example.com/");
        assert_eq!(
            normalize_url("localhost:5173").as_str(),
            "http://localhost:5173/"
        );
        assert_eq!(
            normalize_url("http://foo.test/a").as_str(),
            "http://foo.test/a"
        );
    }

    #[test]
    fn the_epoch_formats_as_the_epoch() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_iso(1_757_376_000), "2025-09-09T00:00:00Z");
    }

    #[test]
    fn the_hash_changes_when_a_width_changes() {
        let a = short_hash(&["640".into(), "768".into()]);
        let b = short_hash(&["640".into(), "769".into()]);
        assert_ne!(a, b);
        assert_eq!(a.len(), 8);
    }

    #[test]
    fn slugs_collapse_runs_of_punctuation() {
        assert_eq!(slugify("Bucknell Be The Ray"), "bucknell-be-the-ray");
        assert_eq!(slugify("/Users/rick/Sites/"), "users-rick-sites");
    }
}
