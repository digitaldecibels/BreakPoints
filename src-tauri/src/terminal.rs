//! Starting an agent session that is already connected to this row.
//!
//! WHY THIS EXISTS
//!
//! Connecting a session was two things to remember: open a terminal in the
//! right folder, and run the right slash command in it. Neither is hard and
//! both are easy to skip, and a row with nothing listening sends its notes to
//! the clipboard instead, which you only find out about afterwards.
//!
//! This runs the agent for you, in the project's folder, in your own terminal.
//! It deliberately does not embed one: a panel is a child webview composited
//! over the chrome, so anything drawn behind the row cannot be seen, and a
//! terminal large enough to use would have to come out of the panels' height.
//! Your own terminal is also better than any terminal this app would grow.
//!
//! THE AWKWARD PART: EVERY TERMINAL STARTS A COMMAND DIFFERENTLY
//!
//! There is no portable way to say "open and run this". Three routes exist and
//! each terminal takes exactly one of them, which is what `Launch` below is:
//!
//! - Terminal.app runs a shell script handed to it as a document. This is the
//!   one that always works, on every Mac, with no permission to grant, which
//!   is why it is first in the list and is what an unset preference picks.
//! - kitty, Alacritty, WezTerm and Ghostty take the command as launch
//!   arguments. Verified against each project's documented flags rather than
//!   on this machine, because none of them were installed to test against.
//! - iTerm takes neither. Its only route is AppleScript, which needs macOS
//!   automation permission, and that permission is tied to the exact binary,
//!   so an unsigned app loses it on every rebuild the same way screen
//!   recording does. It is offered, it is tried with a time limit, and it says
//!   plainly what to do when it does not answer.
//!
//! Warp is deliberately absent. It has no documented way to be told what to
//! run at launch, and a button that opens a terminal without starting the
//! agent is worse than no button.
//!
//! FOCUS
//!
//! Like the browser button, this one is allowed to take focus. You pressed a
//! button that starts a session you are about to type in. `focus` exists for
//! the same reason: the status bar offers to bring that session forward once
//! it has replied to you.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How a terminal is told what to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Launch {
    /// Hand it the script as a document and it runs it.
    Document,
    /// Launch it with arguments that name the script. The slice is everything
    /// before the script's path.
    Args(&'static [&'static str]),
    /// Drive it with AppleScript, because it accepts nothing else.
    AppleScript,
}

/// A terminal we know how to start a command in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Terminal {
    Terminal,
    ITerm,
    Ghostty,
    WezTerm,
    Kitty,
    Alacritty,
}

impl Terminal {
    /// Terminal.app first, because it is the one that is always installed and
    /// always works, so it is the right default for somebody who has never
    /// opened these settings.
    pub const ALL: [Terminal; 6] = [
        Terminal::Terminal,
        Terminal::ITerm,
        Terminal::Ghostty,
        Terminal::WezTerm,
        Terminal::Kitty,
        Terminal::Alacritty,
    ];

    /// The name `open -a` wants, which is also the bundle name.
    pub fn app_name(self) -> &'static str {
        match self {
            Terminal::Terminal => "Terminal",
            Terminal::ITerm => "iTerm",
            Terminal::Ghostty => "Ghostty",
            Terminal::WezTerm => "WezTerm",
            Terminal::Kitty => "kitty",
            Terminal::Alacritty => "Alacritty",
        }
    }

    /// What a person calls it.
    pub fn label(self) -> &'static str {
        match self {
            Terminal::Terminal => "Terminal",
            Terminal::ITerm => "iTerm",
            Terminal::Ghostty => "Ghostty",
            Terminal::WezTerm => "WezTerm",
            Terminal::Kitty => "kitty",
            Terminal::Alacritty => "Alacritty",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Terminal::Terminal => "terminal",
            Terminal::ITerm => "iterm",
            Terminal::Ghostty => "ghostty",
            Terminal::WezTerm => "wezterm",
            Terminal::Kitty => "kitty",
            Terminal::Alacritty => "alacritty",
        }
    }

    pub fn from_id(id: &str) -> Option<Terminal> {
        Terminal::ALL.into_iter().find(|t| t.id() == id)
    }

    fn launch(self) -> Launch {
        match self {
            Terminal::Terminal => Launch::Document,
            Terminal::ITerm => Launch::AppleScript,
            // `-e` is "run this instead of a shell" in both.
            Terminal::Ghostty | Terminal::Alacritty => Launch::Args(&["-e"]),
            Terminal::WezTerm => Launch::Args(&["start", "--"]),
            // kitty takes the program as its trailing arguments, with no flag.
            Terminal::Kitty => Launch::Args(&[]),
        }
    }

    /// True when macOS will ask to let Break/Points control this app the first
    /// time the button is pressed. Said in the settings so a prompt nobody
    /// expected is a prompt somebody was warned about.
    pub fn needs_permission(self) -> bool {
        self.launch() == Launch::AppleScript
    }

    /// Is it on this machine?
    ///
    /// Four folders, because Terminal.app lives in a system one and a terminal
    /// is exactly the kind of thing somebody else keeps in `~/Applications`.
    pub fn installed(self) -> bool {
        app_paths(self.app_name()).into_iter().any(|p| p.exists())
    }
}

fn app_paths(name: &str) -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        // Terminal.app has lived here since macOS moved its bundled apps out
        // of /Applications. Leaving it out made the one terminal that always
        // works look like it was not installed.
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/Applications/Utilities"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
        .into_iter()
        .map(|root| root.join(format!("{name}.app")))
        .collect()
}

/// Every terminal installed, in a fixed order so the list does not reshuffle.
pub fn installed() -> Vec<Terminal> {
    Terminal::ALL.into_iter().filter(|t| t.installed()).collect()
}

/// What to run when nobody has said otherwise.
///
/// The prompt is the skill's own name, so the session connects itself: opening
/// the socket is what claims the reports.
pub const DEFAULT_COMMAND: &str = "claude \"/run-breakpoints\"";

/// How long iTerm gets to answer before we stop waiting on it.
const APPLESCRIPT_PATIENCE: Duration = Duration::from_secs(8);

/// The script a terminal is asked to run.
///
/// Written to a file rather than built into each terminal's arguments because
/// quoting a command differently for five terminals is five ways to get it
/// wrong. It changes directory first so the agent starts in the project, and
/// `exec` means the shell does not linger under the agent.
pub fn script_for(project: Option<&Path>, command: &str) -> String {
    let mut out = String::from("#!/bin/sh\n");
    out.push_str("# Written by Break/Points to start an agent session for this row.\n");
    if let Some(project) = project {
        out.push_str(&format!("cd {} || exit 1\n", shell_quote(&project.to_string_lossy())));
    }
    out.push_str(&format!("exec {command}\n"));
    out
}

/// Single quotes, with any single quote inside them escaped the shell's way.
fn shell_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', r"'\''"))
}

/// Double quotes and backslashes escaped for an AppleScript string literal.
fn applescript_quote(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', r"\\").replace('"', "\\\""))
}

/// Start the agent in the chosen terminal.
pub fn start(
    terminal: Terminal,
    project: Option<&Path>,
    command: &str,
    scripts_dir: &Path,
) -> Result<String, String> {
    if command.trim().is_empty() {
        return Err("there is no command to run. Set one in Settings.".into());
    }
    if !terminal.installed() {
        return Err(format!("{} is not installed on this machine.", terminal.label()));
    }

    let path = write_script(project, command, scripts_dir)?;

    match terminal.launch() {
        Launch::Document => {
            // No `-n` here. Terminal.app opens a new window for a document it
            // is handed, and asking for a second copy of the application gets
            // a warning about running two of them instead.
            run(Command::new("open").arg("-a").arg(terminal.app_name()).arg(&path), terminal)?;
        }
        Launch::Args(before) => {
            // `-n` so a terminal that is already open still gets a new window
            // rather than the arguments being handed to a running copy that
            // ignores them.
            let mut cmd = Command::new("open");
            cmd.arg("-n").arg("-a").arg(terminal.app_name()).arg("--args");
            for arg in before {
                cmd.arg(arg);
            }
            cmd.arg(&path);
            run(&mut cmd, terminal)?;
        }
        Launch::AppleScript => start_with_applescript(terminal, &path)?,
    }

    Ok(terminal.label().to_string())
}

/// Bring a terminal forward without starting anything in it.
///
/// `open -a` with no `-n` activates the copy that is already running, which is
/// the session you were reading about rather than a fresh empty window.
pub fn focus(terminal: Terminal) -> Result<String, String> {
    if !terminal.installed() {
        return Err(format!("{} is not installed on this machine.", terminal.label()));
    }
    run(Command::new("open").arg("-a").arg(terminal.app_name()), terminal)?;
    Ok(terminal.label().to_string())
}

fn write_script(
    project: Option<&Path>,
    command: &str,
    scripts_dir: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(scripts_dir)
        .map_err(|e| format!("could not make {}: {e}", scripts_dir.display()))?;
    // `.command` rather than `.sh`, because that is the extension macOS treats
    // as "a shell script you can run", which is what makes Terminal.app run it
    // instead of opening it in an editor.
    let path = scripts_dir.join("start-agent.command");
    std::fs::write(&path, script_for(project, command))
        .map_err(|e| format!("could not write the start script: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not make the start script runnable: {e}"))?;
    }
    Ok(path)
}

fn run(cmd: &mut Command, terminal: Terminal) -> Result<(), String> {
    cmd.spawn()
        .map_err(|e| format!("could not open {}: {e}", terminal.label()))?;
    Ok(())
}

/// iTerm, the only one that takes neither a document nor arguments.
///
/// Bounded rather than waited on. A first run without automation permission
/// leaves the AppleEvent hanging with nothing to time it out, and an app that
/// freezes when you press a button is worse than one that tells you why.
fn start_with_applescript(terminal: Terminal, script: &Path) -> Result<(), String> {
    let program = applescript_quote(&script.to_string_lossy());
    let source = format!(
        r#"tell application "{app}"
  activate
  set theWindow to (create window with default profile)
  tell current session of theWindow to write text {program}
end tell"#,
        app = terminal.app_name(),
    );

    let mut child = Command::new("osascript")
        .arg("-e")
        .arg(&source)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run osascript: {e}"))?;

    let deadline = Instant::now() + APPLESCRIPT_PATIENCE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                let mut why = String::new();
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_string(&mut why);
                }
                let why = why.trim();
                return Err(format!(
                    "{} refused to start a session{}. macOS has to allow Break/Points to control it: System Settings, Privacy and Security, Automation. Terminal needs no permission and always works.",
                    terminal.label(),
                    if why.is_empty() { String::new() } else { format!(" ({why})") }
                ));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                return Err(format!(
                    "{} did not answer. It is probably waiting for permission to be controlled: look for a macOS prompt, or allow it under Privacy and Security, Automation. Terminal needs no permission and always works.",
                    terminal.label()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("could not wait for osascript: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_changes_directory_and_runs_the_command() {
        let script = script_for(Some(Path::new("/Users/rick/Herd/site")), "claude \"/x\"");
        assert!(script.starts_with("#!/bin/sh"));
        assert!(script.contains("cd '/Users/rick/Herd/site'"));
        assert!(script.trim_end().ends_with("exec claude \"/x\""));
    }

    /// A folder with a quote in its name is not a way to run something else.
    #[test]
    fn a_quote_in_the_path_cannot_escape_the_quoting() {
        let script = script_for(Some(Path::new("/tmp/it's here")), "claude");
        assert!(script.contains(r"cd '/tmp/it'\''s here'"), "{script}");
    }

    #[test]
    fn with_no_project_it_just_runs_the_command() {
        let script = script_for(None, "codex");
        assert!(!script.contains("cd "));
        assert!(script.contains("exec codex"));
    }

    #[test]
    fn ids_round_trip() {
        for terminal in Terminal::ALL {
            assert_eq!(Terminal::from_id(terminal.id()), Some(terminal));
        }
        assert_eq!(Terminal::from_id("nothing"), None);
    }

    /// The default pick is the one that needs nothing granted and is on every
    /// Mac. Getting this order wrong makes the button fail on first press for
    /// somebody who never opened the settings.
    #[test]
    fn terminal_app_is_first_and_needs_no_permission() {
        assert_eq!(Terminal::ALL[0], Terminal::Terminal);
        assert!(!Terminal::Terminal.needs_permission());
        assert!(Terminal::ITerm.needs_permission());
    }

    /// Terminal.app is bundled with macOS and is the fallback everything else
    /// leans on, so its folder has to be one we look in.
    #[test]
    fn terminal_app_is_found_where_macos_keeps_it() {
        assert!(app_paths("Terminal")
            .iter()
            .any(|p| p.starts_with("/System/Applications/Utilities")));
    }

    #[test]
    fn a_quote_in_the_script_path_cannot_escape_the_applescript_string() {
        let quoted = applescript_quote(r#"/tmp/a"b\c"#);
        assert_eq!(quoted, r#""/tmp/a\"b\\c""#);
    }
}
