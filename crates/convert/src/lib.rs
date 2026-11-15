//! Reading the office documents a vault folder already holds into this app's Markdown.
//!
//! Port of the upstream `odt.ts` plus the `.docx` path (mammoth), with one structural difference
//! that makes the whole thing smaller: the upstream converts XML → **HTML** → Markdown (turndown),
//! and this crate goes XML → Markdown directly. There is no HTML pipeline in this app, and the
//! double step lost exactly the structures a note cares about (tables, list nesting).
//!
//! Two entry points, one per format, and [`to_markdown`] to pick by extension:
//!
//! - [`odt`]: OpenDocument Text - `content.xml` (+ `styles.xml` for the styles the body reuses).
//! - [`docx`]: Word - `word/document.xml` (+ the rels for link targets, + `numbering.xml` to tell
//!   a numbered list from a bulleted one).
//!
//! What is deliberately **not** converted, the same as the upstream (its limitations.md #8 keeps a
//! document's images out): pictures, charts, footnotes, comments, text boxes and tracked changes.
//! The text of a dropped picture's caption survives, which is what a reader wants in a note.

pub mod docx;
pub mod odt;
mod text;
mod xml;

use std::fmt;
use std::path::Path;

/// What can go wrong reading a document. Every variant is something the user can be told about.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Zip(zip::result::ZipError),
    /// The XML inside the package is broken or unexpected.
    Xml(String),
    /// The package does not hold the part the format requires.
    Missing(&'static str),
    /// Not a format this crate reads.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "{error}"),
            Error::Zip(error) => write!(f, "{error}"),
            Error::Xml(message) => write!(f, "the document's XML is unreadable: {message}"),
            Error::Missing(part) => write!(f, "the document has no {part}"),
            Error::Unsupported(extension) => write!(f, "\"{extension}\" is not a document this app reads"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(error) => Some(error),
            Error::Zip(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(error: zip::result::ZipError) -> Self {
        Error::Zip(error)
    }
}

/// Reads `path` as a document and returns the note's Markdown.
///
/// The format comes from the extension: `.odt` and `.docx` only. Anything else is refused by name
/// rather than guessed at - opening a random binary as a ZIP would only produce a worse message.
pub fn to_markdown(path: &Path) -> Result<String, Error> {
    let extension = path.extension().map(|value| value.to_string_lossy().to_lowercase()).unwrap_or_default();
    // The format is settled before the file is opened: a bad extension is a better message than
    // "not a zip" from a file that was never a document.
    if extension != "odt" && extension != "docx" {
        return Err(Error::Unsupported(extension));
    }
    let bytes = std::fs::read(path)?;
    if extension == "odt" { odt::from_package(&bytes) } else { docx::from_package(&bytes) }
}

/// The document's lines as a note: a blank line between blocks, one newline at the end. A note that
/// opens with a blank Markdown line looks broken, so trailing blanks are trimmed.
pub(crate) fn join_document(lines: &[String]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push_str("\n\n");
        }
        out.push_str(line.trim_end_matches('\n'));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Whether this crate claims the file, so a caller can leave the other files to the OS.
pub fn is_document(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("odt") || extension.eq_ignore_ascii_case("docx"))
}

/// The entries a package holds, read one by one. Kept here so both formats open a ZIP the same way.
fn read_part(package: &[u8], wanted: &str) -> Result<Option<String>, Error> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(package))?;
    let Ok(mut entry) = archive.by_name(wanted) else { return Ok(None) };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text)?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An office package in memory: a ZIP with the given entries.
    fn package(entries: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, content) in entries {
                writer.start_file(*name, options).expect("the entry starts");
                writer.write_all(content.as_bytes()).expect("the entry is written");
            }
            writer.finish().expect("the package is finished");
        }
        buffer.into_inner()
    }

    #[test]
    fn an_odt_package_is_read_from_its_content_part() {
        let content = "<office:document-content><office:body><office:text>\
                       <text:h text:outline-level=\"1\">Sepse</text:h><text:p>Lactato em 2h.</text:p>\
                       </office:text></office:body></office:document-content>";
        let bytes = package(&[("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)]);
        assert_eq!(odt::from_package(&bytes).expect("the package converts"), "# Sepse\n\nLactato em 2h.\n");
    }

    #[test]
    fn an_odt_without_content_xml_says_which_part_is_missing() {
        let bytes = package(&[("mimetype", "application/vnd.oasis.opendocument.text")]);
        assert!(matches!(odt::from_package(&bytes), Err(Error::Missing("content.xml"))));
    }

    #[test]
    fn a_docx_package_is_read_from_its_document_part() {
        let document = "<w:document><w:body><w:p><w:r><w:t>Doses</w:t></w:r></w:p></w:body></w:document>";
        let bytes = package(&[("[Content_Types].xml", "<Types/>"), ("word/document.xml", document)]);
        assert_eq!(docx::from_package(&bytes).expect("the package converts"), "Doses\n");
    }

    #[test]
    fn a_file_that_is_not_a_document_is_refused_by_name() {
        let error = to_markdown(Path::new("relatorio.pdf")).expect_err("a PDF is not a document here");
        assert!(matches!(error, Error::Unsupported(ref extension) if extension == "pdf"), "{error}");
        assert!(!is_document(Path::new("relatorio.pdf")) && is_document(Path::new("ata.docx")));
    }

    #[test]
    fn the_error_says_what_went_wrong() {
        let bytes = package(&[("content.xml", "<office:document-content>")]);
        // A truncated part still converts (the tree closes what is open).
        assert!(odt::from_package(&bytes).is_ok());
        let error = docx::from_package(&bytes).expect_err("a package without a document");
        assert!(error.to_string().contains("word/document.xml"), "{error}");
    }
}
