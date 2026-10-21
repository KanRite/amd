//! Line endings: amd reads CRLF and writes LF (ticket 277).
//!
//! Git for Windows checks text out with CRLF by default (`core.autocrlf`),
//! and Windows editors save it, so a ticket, a `.settings` or a document
//! can reach amd with `\r\n` endings. Every reader here splits on `\n` and
//! looks for `---\n`, so a CRLF file read as it is loses its frontmatter.
//!
//! The rule is dos2unix's: on the way in, each `\r\n` becomes `\n`; on the
//! way out amd writes what it has, which is then LF. A file written on any
//! system reads the same on every other, and with `autocrlf` git stores LF
//! anyway, so a commit still shows only the lines that changed. A lone
//! `\r` is left alone: it isn't a line ending anyone writes.

use std::io;
use std::path::Path;

/// `text` with each `\r\n` made `\n`, returned as it is when it has none.
pub fn lf(text: String) -> String {
    if text.contains("\r\n") {
        text.replace("\r\n", "\n")
    } else {
        text
    }
}

/// [`std::fs::read_to_string`], with CRLF read as LF.
pub fn read(path: impl AsRef<Path>) -> io::Result<String> {
    std::fs::read_to_string(path).map(lf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_becomes_lf_and_lf_is_left_as_it_is() {
        assert_eq!(
            lf("---\r\nid: \"1\"\r\n---\r\n".into()),
            "---\nid: \"1\"\n---\n"
        );
        assert_eq!(lf("a\nb\n".into()), "a\nb\n");
        assert_eq!(lf("mixed\r\nends\nhere\r\n".into()), "mixed\nends\nhere\n");
    }

    #[test]
    fn a_lone_carriage_return_is_not_a_line_ending() {
        assert_eq!(lf("a\rb\r\n".into()), "a\rb\n");
    }

    #[test]
    fn read_gives_lf_whatever_the_file_has() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.md");
        std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
        assert_eq!(read(&path).unwrap(), "one\ntwo\n");
        assert!(read(dir.path().join("missing")).is_err());
    }
}
