//! YAML frontmatter - the metadata block every Markdown tool shares (Obsidian, Jekyll, Hugo,
//! Logseq). Parsing it is what makes an imported vault readable: without it the `---` block shows
//! up as a stray horizontal rule at the top of the note, and `tags:` as body text.
//!
//! The block counts only when it opens on the very first line (`---` followed by a line break) and
//! closes on a line that holds nothing but `---`. Invalid YAML leaves the note readable: the
//! fields are dropped and the body is returned untouched, because a broken header must never take
//! the content down with it.

use saphyr::{LoadableYamlNode, MappingOwned, ScalarOwned, YamlEmitter, YamlOwned};

/// The fence that opens and closes the block.
const FENCE: &str = "---";

/// A note split into its metadata block and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct Split<'a> {
    /// `None` when there is no block, when its YAML is invalid, or when it is not a mapping.
    pub frontmatter: Option<Frontmatter>,
    /// The note without the block; the whole note when there is no block.
    pub body: &'a str,
}

/// The note's YAML fields, as the file wrote them.
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    yaml: YamlOwned,
}

impl Frontmatter {
    /// An empty mapping (a note that starts with an empty block, or one we are about to write).
    pub fn empty() -> Self {
        Self { yaml: YamlOwned::Mapping(MappingOwned::new()) }
    }

    /// The fields of `yaml_text` (the text between the fences). `None` when it is not a mapping:
    /// a list or a bare scalar is not frontmatter.
    pub fn from_yaml(yaml_text: &str) -> Option<Self> {
        let documents = YamlOwned::load_from_str(yaml_text).ok()?;
        let first = documents.into_iter().next()?;
        let node = untag(first);
        node.is_mapping().then_some(Self { yaml: node })
    }

    /// The field `key`, as written.
    pub fn get(&self, key: &str) -> Option<&YamlOwned> {
        self.yaml.as_mapping_get(key)
    }

    /// `key` as a list of strings: a YAML list, a single value, or a comma-separated string.
    /// Anything else (a nested mapping, a list of mappings) contributes nothing.
    pub fn list(&self, key: &str) -> Vec<String> {
        let Some(value) = self.get(key) else { return Vec::new() };
        let value = untag_ref(value);
        if let Some(sequence) = value.as_sequence() {
            return sequence.iter().filter_map(scalar_text).filter(|item| !item.is_empty()).collect();
        }
        match value.as_str() {
            Some(text) => split_commas(text),
            // A scalar that YAML resolved to a number or a boolean still names something.
            None => scalar_text(value).into_iter().collect(),
        }
    }

    /// Tags declared in the block, without the leading `#`, lowercased. `tags` and `tag` both count.
    pub fn tags(&self) -> Vec<String> {
        let raw = if self.get("tags").is_some() { self.list("tags") } else { self.list("tag") };
        raw.into_iter().map(|tag| tag.trim_start_matches('#').to_lowercase()).filter(|tag| !tag.is_empty()).collect()
    }

    /// Alternative names of the note: `aliases` or `alias`, with their original casing.
    pub fn aliases(&self) -> Vec<String> {
        if self.get("aliases").is_some() { self.list("aliases") } else { self.list("alias") }
    }

    /// Whether the block holds no fields at all.
    pub fn is_empty(&self) -> bool {
        self.yaml.as_mapping().is_none_or(hashlink_is_empty)
    }

    /// Sets `key` to a list of strings (used when a note is created with fields already in it).
    pub fn set_list(&mut self, key: &str, values: &[String]) {
        let sequence: Vec<YamlOwned> = values.iter().map(|v| YamlOwned::Value(ScalarOwned::String(v.clone()))).collect();
        let value = YamlOwned::Sequence(sequence);
        let mapping = match std::mem::replace(&mut self.yaml, Frontmatter::empty().yaml) {
            YamlOwned::Mapping(mut mapping) => {
                mapping.insert(YamlOwned::Value(ScalarOwned::String(key.to_string())), value);
                mapping
            }
            _ => MappingOwned::new(),
        };
        self.yaml = YamlOwned::Mapping(mapping);
    }

    /// The block as it goes back into the file: fences included, ending with a newline. An empty
    /// block renders as an empty string, so a note without fields never grows a stray `---`.
    pub fn to_block(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut yaml = String::new();
        let borrowed: saphyr::Yaml<'_> = (&self.yaml).into();
        let mut emitter = YamlEmitter::new(&mut yaml);
        if emitter.dump(&borrowed).is_err() {
            return String::new();
        }
        // The emitter may lead with a document marker of its own; the block supplies its fences.
        let body = yaml.strip_prefix("---\n").unwrap_or(&yaml);
        let body = body.trim_end_matches('\n');
        format!("{FENCE}\n{body}\n{FENCE}\n")
    }
}

/// A tagged node is the node itself for every purpose here (`!custom 3` is a number).
fn untag(node: YamlOwned) -> YamlOwned {
    match node {
        YamlOwned::Tagged(_, inner) => untag(*inner),
        other => other,
    }
}

fn untag_ref(node: &YamlOwned) -> &YamlOwned {
    match node {
        YamlOwned::Tagged(_, inner) => untag_ref(inner),
        other => other,
    }
}

/// A scalar as the text the user wrote.
fn scalar_text(node: &YamlOwned) -> Option<String> {
    let node = untag_ref(node);
    match node {
        YamlOwned::Value(scalar) => match scalar {
            ScalarOwned::Integer(value) => Some(value.to_string()),
            ScalarOwned::FloatingPoint(value) => Some(value.to_string()),
            ScalarOwned::Boolean(value) => Some(value.to_string()),
            ScalarOwned::String(value) => Some(value.clone()),
            ScalarOwned::Null => None,
        },
        YamlOwned::Representation(text, _, _) => Some(text.clone()),
        _ => None,
    }
}

fn split_commas(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_string).collect()
}

fn hashlink_is_empty(mapping: &MappingOwned) -> bool {
    mapping.is_empty()
}

/// Splits `note` into its frontmatter and its body.
pub fn split(note: &str) -> Split<'_> {
    let Some((yaml_text, body)) = block_span(note) else {
        return Split { frontmatter: None, body: note };
    };
    Split { frontmatter: Frontmatter::from_yaml(yaml_text), body }
}

/// The note's body, without the block.
pub fn body(note: &str) -> &str {
    split(note).body
}

/// The fields of the note, if it has a block.
pub fn parse(note: &str) -> Option<Frontmatter> {
    split(note).frontmatter
}

/// `(yaml text, body)` for a block that opens on the first line and closes on a `---` line.
fn block_span(note: &str) -> Option<(&str, &str)> {
    let after_opener = note.strip_prefix(FENCE)?;
    let opener_end = match after_opener.as_bytes() {
        [b'\n', ..] => FENCE.len() + 1,
        [b'\r', b'\n', ..] => FENCE.len() + 2,
        // `---text` is a line of its own, not a fence.
        _ => return None,
    };

    let mut line_start = opener_end;
    while line_start <= note.len() {
        let line_end = note[line_start..].find('\n').map(|offset| line_start + offset).unwrap_or(note.len());
        let line = &note[line_start..line_end];
        let content = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = content.trim_end_matches([' ', '\t']);
        if trimmed == FENCE {
            let body_start = if line_end < note.len() { line_end + 1 } else { note.len() };
            return Some((&note[opener_end..line_start], &note[body_start..]));
        }
        if line_end >= note.len() {
            break;
        }
        line_start = line_end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(note: &str) -> Frontmatter {
        split(note).frontmatter.expect("a frontmatter block")
    }

    #[test]
    fn splits_the_fields_from_the_body() {
        let split = split("---\ntitulo: Sepse\n---\n# Sepse\n\ntexto");
        assert_eq!(split.body, "# Sepse\n\ntexto");
        assert_eq!(split.frontmatter.expect("fields").get("titulo").and_then(|v| v.as_str()), Some("Sepse"));
    }

    #[test]
    fn reads_inline_and_block_lists() {
        let note = "---\ntags: [cardio, uti]\naliases:\n  - IAM\n  - infarto\n---\ncorpo";
        let frontmatter = fields(note);
        assert_eq!(frontmatter.tags(), vec!["cardio", "uti"]);
        assert_eq!(frontmatter.aliases(), vec!["IAM", "infarto"]);
    }

    #[test]
    fn a_block_that_does_not_start_the_note_is_not_frontmatter() {
        let note = "texto\n\n---\ntags: x\n---\n";
        let split = split(note);
        assert!(split.frontmatter.is_none());
        assert_eq!(split.body, note);
    }

    #[test]
    fn the_body_survives_invalid_yaml() {
        let split = split("---\n: : :\n\tx\n---\ncorpo");
        assert!(split.frontmatter.is_none());
        assert_eq!(split.body, "corpo");
    }

    #[test]
    fn a_note_without_a_block_is_returned_whole() {
        let note = "# Só um título";
        let split = split(note);
        assert!(split.frontmatter.is_none());
        assert_eq!(split.body, note);
    }

    #[test]
    fn an_empty_block_has_no_fields() {
        let split = split("---\n\n---\ncorpo");
        assert_eq!(split.body, "corpo");
        assert!(split.frontmatter.is_none_or(|f| f.is_empty()));
    }

    #[test]
    fn crlf_line_endings_work() {
        let split = split("---\r\ntags: [a]\r\n---\r\ncorpo");
        assert_eq!(split.body, "corpo");
        assert_eq!(split.frontmatter.expect("fields").tags(), vec!["a"]);
    }

    #[test]
    fn tags_lose_the_hash_and_the_case() {
        let frontmatter = fields("---\ntags: ['#Cardio', UTI]\n---\n");
        assert_eq!(frontmatter.tags(), vec!["cardio", "uti"]);
    }

    #[test]
    fn the_singular_keys_count_too() {
        assert_eq!(fields("---\ntag: cardio\n---\n").tags(), vec!["cardio"]);
        assert_eq!(fields("---\nalias: SCA\n---\n").aliases(), vec!["SCA"]);
    }

    #[test]
    fn a_comma_separated_string_is_a_list() {
        assert_eq!(fields("---\ntags: cardio, uti\n---\n").tags(), vec!["cardio", "uti"]);
    }

    #[test]
    fn a_block_round_trips() {
        let frontmatter = fields("---\ntags:\n  - cardio\n  - uti\ntitulo: Sepse\n---\ncorpo");
        let block = frontmatter.to_block();
        assert!(block.starts_with("---\n"), "{block}");
        assert!(block.ends_with("---\n"), "{block}");
        let again = Frontmatter::from_yaml(&block[4..block.len() - 4]).expect("re-parsed");
        assert_eq!(again.tags(), vec!["cardio", "uti"]);
        assert_eq!(again.get("titulo").and_then(|v| v.as_str()), Some("Sepse"));
    }

    #[test]
    fn an_empty_block_renders_as_nothing() {
        assert_eq!(Frontmatter::empty().to_block(), "");
    }

    #[test]
    fn set_list_writes_a_block_that_parses_back() {
        let mut frontmatter = Frontmatter::empty();
        frontmatter.set_list("tags", &["cardio".to_string(), "uti".to_string()]);
        let block = frontmatter.to_block();
        let parsed = Frontmatter::from_yaml(&block[4..block.len() - 4]).expect("re-parsed");
        assert_eq!(parsed.tags(), vec!["cardio", "uti"]);
    }

    #[test]
    fn a_yaml_list_is_not_frontmatter() {
        assert!(Frontmatter::from_yaml("- a\n- b\n").is_none());
    }
}
