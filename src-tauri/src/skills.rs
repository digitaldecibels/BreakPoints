//! The skills an agent session can be asked to run, and sending it one.
//!
//! WHY THIS EXISTS
//!
//! A skill is a packaged workflow: a folder with a `SKILL.md` in it, invoked by
//! typing its name with a slash in front. That is fine if you wrote it and
//! remember it exists. It is useless to somebody looking at a row of panels who
//! knows what they want done and not what it is called, which after a few weeks
//! is also you.
//!
//! So the app reads the skills that are actually installed, shows them with the
//! description their own author wrote, and sends the chosen one to whichever
//! session is listening. Nothing to remember and nothing to type.
//!
//! HOW IT REACHES THE SESSION
//!
//! Down the same socket a note goes down. A session holding `/ws/reports` open
//! is already reading text frames and acting on them, so a frame saying "run
//! this skill" needs no new transport, no new port and no new client code. That
//! is the whole reason this is cheap.
//!
//! WHAT IT DOES NOT DO
//!
//! It does not run anything itself. The app cannot execute a skill: a skill is
//! instructions for an agent, and the agent is in a terminal the app does not
//! own. This asks. If nothing is listening, it says so rather than pretending.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One installed skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// The name it is invoked by, which is the folder name unless the file
    /// says otherwise.
    pub name: String,
    /// The author's own one-line description. Shown in the menu, because a
    /// list of slugs is not a menu.
    pub description: String,
    /// "yours" for one installed for every project, or the project's own name.
    /// Worth showing: a skill that only exists here behaves differently from
    /// one that follows you around.
    pub scope: String,
    pub path: String,
}

/// Read the frontmatter of one `SKILL.md`.
///
/// Deliberately not a YAML parser. The frontmatter of a skill is two or three
/// flat keys, and pulling in a parser to read `name:` would be a dependency
/// that can fail in more ways than the thing it replaces. Anything it cannot
/// understand falls back to the folder name, so a malformed file is a skill
/// with a poor description rather than a skill that vanishes.
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut description = None;

    let mut lines = text.lines();
    // The frontmatter has to be the first thing in the file.
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }

    let mut key: Option<&str> = None;
    let mut value = String::new();

    for line in lines {
        if line.trim() == "---" {
            break;
        }
        // A description long enough to wrap is folded onto continuation lines,
        // which are indented and belong to the key above them.
        let is_continuation = line.starts_with(' ') || line.starts_with('\t');
        if is_continuation && key.is_some() {
            if !value.is_empty() {
                value.push(' ');
            }
            value.push_str(line.trim());
            continue;
        }

        // A finished key goes in before the next one starts.
        if let Some(k) = key.take() {
            match k {
                "name" => name = Some(value.clone()),
                "description" => description = Some(value.clone()),
                _ => {}
            }
        }
        value.clear();

        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        if k == "name" || k == "description" {
            key = Some(if k == "name" { "name" } else { "description" });
            value.push_str(v.trim());
        }
    }
    if let Some(k) = key {
        match k {
            "name" => name = Some(value.clone()),
            "description" => description = Some(value),
            _ => {}
        }
    }

    let tidy = |s: Option<String>| {
        s.map(|s| s.trim().trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
    };
    (tidy(name), tidy(description))
}

/// Read every skill in one folder of skill folders.
fn read_dir(dir: &Path, scope: &str, out: &mut Vec<Skill>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file = path.join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        // Only the top of the file is read. A skill is a long document and the
        // frontmatter is the first dozen lines of it.
        let head: String = text.chars().take(4000).collect();
        let (name, description) = parse_frontmatter(&head);
        let folder = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(Skill {
            name: name.unwrap_or_else(|| folder.clone()),
            description: description.unwrap_or_default(),
            scope: scope.to_string(),
            path: file.to_string_lossy().to_string(),
        });
    }
}

/// Every skill installed, the user's own first, then the project's.
///
/// A project can override a skill of the same name, so the project's copy wins
/// and the user's is dropped. Showing both would offer a choice that does not
/// exist: typing the name gets the project's either way.
pub fn installed(project: Option<&Path>) -> Vec<Skill> {
    let mut out = Vec::new();

    if let Some(home) = std::env::var_os("HOME") {
        read_dir(&PathBuf::from(home).join(".claude/skills"), "yours", &mut out);
    }

    if let Some(root) = project {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "this project".into());
        let mut here = Vec::new();
        read_dir(&root.join(".claude/skills"), &name, &mut here);
        // The project's copy of a name replaces the user's.
        out.retain(|skill| !here.iter().any(|theirs| theirs.name == skill.name));
        out.extend(here);
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// What to say to a session to get a skill run.
///
/// Written as a sentence rather than as a bare slash command, because the frame
/// arrives as a notification rather than as something typed at a prompt, and a
/// lone "/compare-to-figma" in a notification reads like a fragment. The name is
/// still in there verbatim so there is no ambiguity about which one is meant.
pub fn invocation(skill: &str, context: Option<&str>) -> String {
    let mut out = format!(
        "Rick asked for this from the Break/Points window: run the {skill} skill now (/{skill})."
    );
    if let Some(context) = context.map(str::trim).filter(|c| !c.is_empty()) {
        out.push_str(&format!("\n\nWhat he added: {context}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_skill_file_is_read() {
        let (name, description) = parse_frontmatter(
            "---\nname: breakpoint-audit\ndescription: Report where things disagree.\n---\n\n# Title\n",
        );
        assert_eq!(name.as_deref(), Some("breakpoint-audit"));
        assert_eq!(description.as_deref(), Some("Report where things disagree."));
    }

    /// A long description wraps onto indented lines, and reading only the first
    /// of them would show half a sentence in the menu.
    #[test]
    fn a_folded_description_is_joined_back_up() {
        let (_, description) = parse_frontmatter(
            "---\nname: x\ndescription: The first part\n  and the second part\n  and the third.\n---\n",
        );
        assert_eq!(
            description.as_deref(),
            Some("The first part and the second part and the third.")
        );
    }

    /// A description with a colon in it is not two keys.
    #[test]
    fn a_colon_inside_a_value_is_left_alone() {
        let (_, description) =
            parse_frontmatter("---\nname: x\ndescription: Use when: something happens.\n---\n");
        assert_eq!(description.as_deref(), Some("Use when: something happens."));
    }

    /// No frontmatter is not an error; the folder name is still a usable name.
    #[test]
    fn a_file_with_no_frontmatter_gives_nothing_rather_than_guessing() {
        let (name, description) = parse_frontmatter("# Just a heading\n\nSome prose.\n");
        assert_eq!(name, None);
        assert_eq!(description, None);
    }

    #[test]
    fn the_invocation_names_the_skill_both_ways() {
        let said = invocation("compare-to-figma", None);
        assert!(said.contains("compare-to-figma skill"));
        assert!(said.contains("/compare-to-figma"));
        assert!(!said.contains("What he added"));
    }

    #[test]
    fn anything_typed_alongside_it_is_carried() {
        let said = invocation("compare-to-figma", Some("  just the hero  "));
        assert!(said.contains("What he added: just the hero"));
    }

    /// Whitespace is not context, and an empty box should not add an empty
    /// line that reads as though something was cut off.
    #[test]
    fn blank_context_is_left_out() {
        assert!(!invocation("x", Some("   ")).contains("What he added"));
    }
}
