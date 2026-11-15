//! Text that goes into a note has to survive Markdown: what the document wrote as literal
//! characters must not come back as syntax. Both formats share this.

/// Wraps `text` in the Markdown its emphasis calls for, keeping any leading/trailing whitespace
/// outside the markers: a `**` that sits after a space does not close, it shows as literal stars.
pub(crate) fn mark(text: &str, bold: bool, italic: bool) -> String {
    if !bold && !italic {
        return text.to_string();
    }
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if start >= end {
        return text.to_string();
    }
    let (lead, body, trail) = (&text[..start], &text[start..end], &text[end..]);
    let markers = if bold && italic {
        "***"
    } else if bold {
        "**"
    } else {
        "*"
    };
    format!("{lead}{markers}{body}{markers}{trail}")
}

/// Escapes the characters that would otherwise turn note text into Markdown syntax, plus the ones
/// that would turn a line into a heading, a quote or a list.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    // The line-start markers and `1.` are decided on the text as written, then applied after the
    // immutable borrow of the buffer ends.
    let trimmed = out.trim_start();
    let indent = out.len() - trimmed.len();
    let blocker = trimmed.chars().next().filter(|first| matches!(first, '#' | '>' | '-' | '+' | '|' | '='));
    let list_dot = trimmed.find(". ").filter(|dot| *dot > 0 && trimmed[..*dot].chars().all(|c| c.is_ascii_digit()));
    if blocker.is_some() {
        out.insert(indent, '\\');
    }
    if let Some(dot) = list_dot {
        out.replace_range(indent + dot..indent + dot + 1, "\\.");
    }
    out
}
