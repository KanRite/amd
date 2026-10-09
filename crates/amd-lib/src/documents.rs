//! A project's documents (ticket 129): the `{project}-documents` repository
//! `amd init` makes beside the tickets one (ADRs, RFCs, docs), and one
//! documents repository shared by every project, named in the project's
//! `.settings` (`shared-documents`). Read only: documents are reviewed
//! through pull requests, so writing one is a branch and a PR, not a save.
//!
//! Every read is confined to its repository: a path is relative, made of
//! plain names (no `..`, no root), and must still be inside the repository
//! once symbolic links are followed.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::board::Board;
use crate::board_settings;
use crate::current_project;
use crate::registry::{self, Registry};

/// The largest file amd reads for the reader: documents and the images
/// they show. Anything bigger is refused rather than loaded.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;

/// Which of a project's documents repositories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The project's own `{project}-documents`.
    Own,
    /// The one every project shares (`shared-documents`).
    Shared,
}

/// A documents repository, found on this machine or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub kind: Kind,
    /// The repository's name.
    pub name: String,
    /// Its clone here; `None` when it isn't cloned on this machine.
    pub root: Option<PathBuf>,
}

/// One document in a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Relative to the repository, with `/` between parts.
    pub path: String,
    /// The first `# ` heading, else the file name without `.md`.
    pub title: String,
}

/// The documents repositories of the project whose board is `board`: its
/// own first, then the shared one when the project names one (and it isn't
/// the project's own).
pub fn sources(board: &Board) -> Vec<Source> {
    let root = board.repo_root();
    let project = current_project::project_name(root);
    let dir = registry::project_dir_of(root);
    let registry = Registry::load();
    let own = format!("{project}-documents");
    let mut out = vec![Source {
        kind: Kind::Own,
        root: locate(&registry, &own, &dir),
        name: own.clone(),
    }];
    if let Some(shared) = board_settings::load(board).shared_documents()
        && shared != own
    {
        out.push(Source {
            kind: Kind::Shared,
            root: locate(&registry, &shared, &dir),
            name: shared,
        });
    }
    out
}

/// Where `name` is cloned for the project in `dir`: beside the project's
/// other repositories, else registered on this machine (`find_near`).
fn locate(registry: &Registry, name: &str, dir: &Path) -> Option<PathBuf> {
    let sibling = dir.join(name);
    if sibling.join(".git").exists() {
        return Some(sibling);
    }
    registry
        .find_near(name, dir)
        .map(|entry| entry.root.clone())
        .filter(|root| root.is_dir())
}

/// Every markdown document in `root`, sorted by path. Hidden files and
/// folders (`.git`, `.github`, `.settings`) are left out.
pub fn list(root: &Path) -> Result<Vec<Document>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Document>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // Not followed: a link could lead out of the repository, or round
        // in a circle.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() && is_markdown(&name) {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let relative = slashed(relative);
            out.push(Document {
                title: title_of(&path).unwrap_or_else(|| stem(&name)),
                path: relative,
            });
        }
    }
    Ok(())
}

fn is_markdown(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

fn stem(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(name)
        .to_string()
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// A document's title: its first `# ` heading, skipping frontmatter.
fn title_of(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    title_in(&text)
}

fn title_in(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let mut in_frontmatter = false;
    let mut first = true;
    for line in lines.by_ref() {
        let trimmed = line.trim();
        if first && trimmed == "---" {
            in_frontmatter = true;
            first = false;
            continue;
        }
        first = false;
        if in_frontmatter {
            if trimmed == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim().trim_end_matches('#').trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

/// `relative` inside `root`, refused when it could reach outside it: an
/// absolute path, a `..`, or a symbolic link that leads elsewhere.
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("{relative:?} isn't a path inside the documents repository");
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("reading {}", root.display()))?;
    let full = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("{relative} isn't in {}", root.display()))?;
    if !full.starts_with(&root) {
        bail!("{relative} leads outside the documents repository");
    }
    if full
        .strip_prefix(&root)
        .is_ok_and(|inside| inside.components().any(|part| part.as_os_str() == ".git"))
    {
        bail!("{relative} is inside .git");
    }
    Ok(full)
}

/// The bytes of `relative` in `root` (a document or an image it shows),
/// confined as [`resolve`] says and no bigger than [`MAX_FILE`].
pub fn read_bytes(root: &Path, relative: &str) -> Result<Vec<u8>> {
    let full = resolve(root, relative)?;
    let size = fs::metadata(&full)
        .with_context(|| format!("reading {relative}"))?
        .len();
    if size > MAX_FILE {
        bail!("{relative} is too big to show ({size} bytes)");
    }
    fs::read(&full).with_context(|| format!("reading {relative}"))
}

/// The text of the document `relative` in `root`.
pub fn read(root: &Path, relative: &str) -> Result<String> {
    if !is_markdown(relative) {
        bail!("{relative} isn't a markdown document");
    }
    let bytes = read_bytes(root, relative)?;
    String::from_utf8(bytes).with_context(|| format!("{relative} isn't UTF-8 text"))
}

/// Where a link in the document `from` points, inside the repository:
/// `../adr/0001.md` from `rfc/0002.md` is `adr/0001.md`. `None` for a link
/// that leaves the repository, or isn't a relative path at all (a web
/// address, `mailto:`, `#a-heading`).
pub fn join_link(from: &str, link: &str) -> Option<String> {
    let link = link.split(['#', '?']).next().unwrap_or_default();
    if link.is_empty() || link.contains(':') || link.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_skip_frontmatter() {
        assert_eq!(
            title_in("---\ntitle: x\n---\n\n# Real title #\n").as_deref(),
            Some("Real title")
        );
        assert_eq!(title_in("no heading\n## Sub\n"), None);
    }

    #[test]
    fn links_resolve_inside_the_repository() {
        assert_eq!(
            join_link("rfc/0002.md", "../adr/0001.md#why").as_deref(),
            Some("adr/0001.md")
        );
        assert_eq!(
            join_link("README.md", "./docs/a.md").as_deref(),
            Some("docs/a.md")
        );
        assert_eq!(join_link("README.md", "../elsewhere.md"), None);
        assert_eq!(join_link("README.md", "https://example.com/a.md"), None);
        assert_eq!(join_link("README.md", "#top"), None);
        assert_eq!(join_link("README.md", "/etc/passwd"), None);
    }

    #[test]
    fn lists_markdown_and_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("adr")).unwrap();
        fs::create_dir_all(root.join(".github")).unwrap();
        fs::write(root.join("README.md"), "# Read me\n").unwrap();
        fs::write(root.join("adr/0001-x.md"), "text\n").unwrap();
        fs::write(root.join("adr/notes.txt"), "not markdown\n").unwrap();
        fs::write(root.join(".github/x.md"), "# hidden\n").unwrap();
        let docs = list(root).unwrap();
        let paths: Vec<&str> = docs.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, ["README.md", "adr/0001-x.md"]);
        assert_eq!(docs[0].title, "Read me");
        assert_eq!(docs[1].title, "0001-x");
    }

    #[test]
    fn reads_stay_inside() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("docs");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("a.md"), "# A\n").unwrap();
        fs::write(root.join(".git/config"), "secret\n").unwrap();
        fs::write(outer.path().join("secret.md"), "no\n").unwrap();
        assert_eq!(read(&root, "a.md").unwrap(), "# A\n");
        assert!(read(&root, "../secret.md").is_err());
        assert!(read(&root, "/etc/passwd").is_err());
        assert!(read_bytes(&root, ".git/config").is_err());
        assert!(read(&root, "missing.md").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outer.path().join("secret.md"), root.join("link.md"))
                .unwrap();
            assert!(read(&root, "link.md").is_err());
        }
    }
}
