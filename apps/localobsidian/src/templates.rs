//! Note templates: a starting point that knows the date and the note's title.
//!
//! Port of the upstream `templateVars.ts` and the template picker. The user's own templates are
//! notes under `_templates/` (the same convention as `_attachments/`), so they are plain files the
//! vault already indexes - no new state, nothing to import.
//!
//! Placeholders: `{{title}}`, `{{date}}`, `{{time}}`, `{{datetime}}` and a custom shape such as
//! `{{date:DD/MM/YYYY}}`. An unknown placeholder stays visible: better a `{{foo}}` the author can
//! see than silently eating what they wrote.

use chrono::{DateTime, Datelike, Local, Timelike};

/// The vault folder holding the user's own templates.
pub const TEMPLATES_DIR: &str = "_templates";

/// Whether a note is one of the user's templates.
pub fn is_template_path(relative_path: &str) -> bool {
    relative_path.replace('\\', "/").to_lowercase().starts_with(&format!("{TEMPLATES_DIR}/"))
}

/// Expands the placeholders in `text`.
pub fn expand_vars(text: &str, title: &str, now: DateTime<Local>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            out.push_str(&rest[open..]);
            return out;
        };
        let token = after[..close].trim();
        let (name, pattern) = match token.split_once(':') {
            Some((name, pattern)) => (name.trim().to_lowercase(), Some(pattern.trim())),
            None => (token.to_lowercase(), None),
        };
        match name.as_str() {
            "title" => out.push_str(title),
            "date" => out.push_str(&format_date(now, pattern.unwrap_or("YYYY-MM-DD"))),
            "time" => out.push_str(&format_date(now, pattern.unwrap_or("HH:mm"))),
            "datetime" => out.push_str(&format_date(now, "YYYY-MM-DD HH:mm")),
            // Unknown: leave what the author wrote.
            _ => {
                out.push_str("{{");
                out.push_str(&after[..close]);
                out.push_str("}}");
            }
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// The tokens a note template actually needs - no date library, no locale data.
fn format_date(now: DateTime<Local>, pattern: &str) -> String {
    let tokens = [
        ("YYYY", format!("{:04}", now.year())),
        ("MM", format!("{:02}", now.month())),
        ("DD", format!("{:02}", now.day())),
        ("HH", format!("{:02}", now.hour())),
        ("mm", format!("{:02}", now.minute())),
        ("ss", format!("{:02}", now.second())),
    ];
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(first) = rest.chars().next() {
        match tokens.iter().find(|(token, _)| rest.starts_with(token)) {
            Some((token, value)) => {
                out.push_str(value);
                rest = &rest[token.len()..];
            }
            None => {
                out.push(first);
                rest = &rest[first.len_utf8()..];
            }
        }
    }
    out
}

/// A built-in template the picker offers.
pub struct Builtin {
    pub id: &'static str,
    pub label: &'static str,
    /// The note's text, with the placeholders already in it.
    pub body: &'static str,
}

/// The built-in templates, in the order the picker shows them.
pub fn builtins() -> &'static [Builtin] {
    &[
        Builtin { id: "blank", label: "Blank", body: "# {{title}}\n\n" },
        Builtin { id: "daily", label: "Daily note", body: "# {{title}}\n\n{{date}}\n\n## Notes\n\n\n\n## Tasks\n\n- [ ] \n- [ ] \n" },
        Builtin {
            id: "meeting",
            label: "Meeting",
            body: "# {{title}}\n\n**Date:** {{date}}\n**Attendees:** \n\n## Agenda\n\n- \n\n## Notes\n\n\n\n## Action items\n\n- [ ] \n",
        },
        Builtin {
            id: "project",
            label: "Project",
            body: "# {{title}}\n\n## Overview\n\n\n\n## Goals\n\n- \n\n## Tasks\n\n- [ ] \n- [ ] \n\n## Notes\n\n\n\n## Resources\n\n- \n",
        },
    ]
}

/// The text of the built-in `id`, with the placeholders expanded.
pub fn expand_builtin(id: &str, title: &str, now: DateTime<Local>) -> String {
    let template = builtins().iter().find(|builtin| builtin.id == id).unwrap_or(&builtins()[0]);
    expand_vars(template.body, title, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 21, 9, 5, 3).single().expect("a valid local time")
    }

    #[test]
    fn the_title_is_replaced() {
        assert_eq!(expand_vars("# {{title}}", "Sepse", now()), "# Sepse");
    }

    #[test]
    fn the_date_and_the_time_are_replaced() {
        assert_eq!(expand_vars("{{date}} {{time}}", "", now()), "2026-07-21 09:05");
    }

    #[test]
    fn datetime_is_the_two_together() {
        assert_eq!(expand_vars("{{datetime}}", "", now()), "2026-07-21 09:05");
    }

    #[test]
    fn a_custom_date_pattern_is_accepted() {
        assert_eq!(expand_vars("{{date:DD/MM/YYYY}}", "", now()), "21/07/2026");
    }

    #[test]
    fn spaces_inside_the_braces_are_tolerated() {
        assert_eq!(expand_vars("{{ title }}", "X", now()), "X");
    }

    #[test]
    fn the_placeholder_name_ignores_case() {
        assert_eq!(expand_vars("{{TITLE}}", "X", now()), "X");
    }

    #[test]
    fn an_unknown_placeholder_stays_visible() {
        assert_eq!(expand_vars("{{foo}}", "X", now()), "{{foo}}");
    }

    #[test]
    fn every_occurrence_is_replaced() {
        assert_eq!(expand_vars("{{title}}/{{title}}", "A", now()), "A/A");
    }

    #[test]
    fn a_template_lives_under_the_templates_folder() {
        assert!(is_template_path("_templates/Plantão.md"));
        assert!(is_template_path("_templates\\Plantão.md"));
        assert!(!is_template_path("Patologias/Sepse.md"));
        assert!(!is_template_path("_templatesX/a.md"));
    }

    #[test]
    fn a_builtin_expands_its_own_title() {
        let meeting = expand_builtin("meeting", "Sprint", now());
        assert!(meeting.starts_with("# Sprint"), "{meeting}");
        assert!(meeting.contains("**Date:** 2026-07-21"), "{meeting}");
        assert!(meeting.contains("## Action items"), "{meeting}");
    }
}
