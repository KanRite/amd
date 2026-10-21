//! `amd renumber` (ticket 146): two tickets with the same number. With the
//! `local` numbering provider, two clones creating tickets while apart can
//! hand out the same number; `sync` catches most of these (ticket 102), but
//! a board can still end up with a clash, from an import, a hand-made file,
//! or a merge done outside amd.
//!
//! One ticket keeps the number: the one committed first, unless told
//! otherwise. Each other one gets the next free number: its file renamed
//! (`git mv`, so its history follows) and its `id` rewritten. References are
//! the hard part. A full `[[042-slug]]` link names one ticket, so it's
//! simply rewritten. A bare `042` in `parent` or `related` could mean either,
//! so it's changed only when the moved ticket's own links prove which one it
//! meant; anything else is reported for a person to check, never guessed.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::board::Board;
use crate::git;
use crate::task::{self, Task};

/// One number held by more than one ticket, and what to do about it.
#[derive(Debug)]
pub struct Clash {
    pub id: u32,
    /// Keeps the number.
    pub keep: Task,
    /// Each moves to its new number.
    pub moves: Vec<(Task, u32)>,
}

/// What [`apply`] did.
#[derive(Debug, Default)]
pub struct Report {
    /// `(old stem, new stem)` for each renamed ticket.
    pub renamed: Vec<(String, String)>,
    /// `(ticket, what changed)` for each reference rewritten.
    pub updated: Vec<(String, String)>,
    /// `(ticket, what to check)` for each reference that could mean either
    /// ticket, left as it was.
    pub ambiguous: Vec<(String, String)>,
}

/// Every clash on `board`, each with the ticket that keeps the number and
/// the new numbers the others get. `keep` names a ticket (its stem, or a
/// unique part of it) to keep its number instead of the first committed.
/// Touches nothing.
pub fn plan(board: &Board, keep: Option<&str>) -> Result<Vec<Clash>> {
    let mut by_id: BTreeMap<u32, Vec<Task>> = BTreeMap::new();
    for task in board.tasks()? {
        if let Some(id) = task.id {
            by_id.entry(id).or_default().push(task);
        }
    }
    let mut next = board.next_id()?;
    let mut clashes = Vec::new();
    let mut kept_by_request = false;
    for (id, mut tickets) in by_id.into_iter().filter(|(_, t)| t.len() > 1) {
        let wanted = keep.and_then(|want| {
            let hits: Vec<usize> = (0..tickets.len())
                .filter(|&i| tickets[i].stem.contains(want))
                .collect();
            (hits.len() == 1).then(|| hits[0])
        });
        let at = match wanted {
            Some(at) => {
                kept_by_request = true;
                at
            }
            None => {
                let repo = board.repo_root();
                (0..tickets.len())
                    .min_by_key(|&i| {
                        (
                            first_committed(repo, &tickets[i].path),
                            tickets[i].meta("created").unwrap_or_default(),
                            tickets[i].stem.clone(),
                        )
                    })
                    .unwrap_or(0)
            }
        };
        let kept = tickets.remove(at);
        let moves = tickets
            .into_iter()
            .map(|t| {
                let to = next;
                next += 1;
                (t, to)
            })
            .collect();
        clashes.push(Clash {
            id,
            keep: kept,
            moves,
        });
    }
    if let Some(want) = keep
        && !kept_by_request
    {
        bail!("--keep {want} doesn't name exactly one of the clashing tickets");
    }
    Ok(clashes)
}

/// Carries out `clashes` (from [`plan`]): renames each moving ticket to its
/// new number and rewrites the references that can be told apart. Refused
/// up front, changing nothing, if a ticket it would change is leased by
/// someone else. Writes files only; committing is the caller's.
pub fn apply(board: &Board, clashes: &[Clash]) -> Result<Report> {
    let all = board.tasks()?;
    for clash in clashes {
        for (moving, _) in &clash.moves {
            moving.ensure_not_leased()?;
        }
    }
    let mut report = Report::default();
    for clash in clashes {
        for (moving, to) in &clash.moves {
            renumber_one(board, &all, clash, moving, *to, &mut report)?;
        }
    }
    Ok(report)
}

fn renumber_one(
    board: &Board,
    all: &[Task],
    clash: &Clash,
    moving: &Task,
    to: u32,
    report: &mut Report,
) -> Result<()> {
    let old_stem = moving.stem.clone();
    let slug = old_stem.split_once('-').map_or("", |(_, slug)| slug);
    let new_stem = format!("{to:03}-{slug}");
    let new_path = moving.path.with_file_name(format!("{new_stem}.md"));
    if new_path.exists() {
        bail!("{} already exists", new_path.display());
    }

    // What the moving ticket's own links say it points at, and what the
    // kept one's say: a reference only one of them accounts for is theirs.
    let moving_text = read(&moving.path)?;
    let kept_text = read(&clash.keep.path)?;
    let moving_children = children(&moving_text);
    let kept_children = children(&kept_text);
    let moving_related = list_ids(&moving_text, "related");
    let kept_related = list_ids(&kept_text, "related");
    let (moving_clones, kept_clones) = (
        list_ids(&moving_text, "clones"),
        list_ids(&kept_text, "clones"),
    );
    let (moving_clone_of, kept_clone_of) = (
        task::meta_in(&moving_text, "clone-of").and_then(number),
        task::meta_in(&kept_text, "clone-of").and_then(number),
    );

    let (moving_links, kept_links) = (link_entries(&moving_text), link_entries(&kept_text));

    let text = task::set_meta(&moving_text, "id", &format!("\"{to:03}\""))?;
    if git::is_tracked(board.repo_root(), &moving.path) {
        git::mv(board.repo_root(), &moving.path, &new_path)?;
    } else {
        fs::rename(&moving.path, &new_path)
            .with_context(|| format!("renaming {}", moving.path.display()))?;
    }
    fs::write(&new_path, text).with_context(|| format!("writing {}", new_path.display()))?;
    board.record_id(to)?;
    report.renamed.push((old_stem.clone(), new_stem.clone()));

    let (old, new) = (format!("{:03}", clash.id), format!("{to:03}"));
    for other in all {
        if other.stem == old_stem || !other.path.exists() {
            continue;
        }
        let before = read(&other.path)?;
        let mut text = before.clone();

        // A full link names exactly one ticket.
        let link = format!("[[{old_stem}]]");
        if text.contains(&link) {
            text = text.replace(&link, &format!("[[{new_stem}]]"));
            report.updated.push((
                other.stem.clone(),
                format!("link [[{old_stem}]] -> [[{new_stem}]]"),
            ));
        }

        if other.stem != clash.keep.stem {
            let ours = |mine: bool, theirs: bool| match (mine, theirs) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                _ => None,
            };
            if task::meta_in(&text, "parent").map(number) == Some(Some(clash.id)) {
                match ours(
                    moving_children.contains(&other.stem),
                    kept_children.contains(&other.stem),
                ) {
                    Some(true) => {
                        text = task::set_meta(&text, "parent", &format!("{new:?}"))?;
                        report
                            .updated
                            .push((other.stem.clone(), format!("parent {old} -> {new}")));
                    }
                    Some(false) => {}
                    None => report.ambiguous.push((
                        other.stem.clone(),
                        format!("parent {old}: {} or {new_stem}?", clash.keep.stem),
                    )),
                }
            }
            let other_id = other.id;
            let related: Vec<String> = task::meta_in(&text, "related")
                .map(task::parse_list)
                .unwrap_or_default();
            if related.iter().any(|r| number(r) == Some(clash.id)) {
                let mine = other_id.is_some_and(|id| moving_related.contains(&id));
                let theirs = other_id.is_some_and(|id| kept_related.contains(&id));
                match ours(mine, theirs) {
                    Some(true) => {
                        let mut list: Vec<String> = related
                            .into_iter()
                            .map(|r| {
                                if number(&r) == Some(clash.id) {
                                    new.clone()
                                } else {
                                    r
                                }
                            })
                            .collect();
                        list.sort();
                        list.dedup();
                        text = task::set_meta(&text, "related", &format!("[{}]", list.join(",")))?;
                        report
                            .updated
                            .push((other.stem.clone(), format!("related {old} -> {new}")));
                    }
                    Some(false) => {}
                    None => report.ambiguous.push((
                        other.stem.clone(),
                        format!("related {old}: {} or {new_stem}?", clash.keep.stem),
                    )),
                }
            }

            // A copy names its original in `clone-of` and the original lists
            // the copy in `clones`, so each end vouches for the other.
            if task::meta_in(&text, "clone-of").and_then(number) == Some(clash.id) {
                let mine = other_id.is_some_and(|id| moving_clones.contains(&id));
                let theirs = other_id.is_some_and(|id| kept_clones.contains(&id));
                match ours(mine, theirs) {
                    Some(true) => {
                        text = task::set_meta(&text, "clone-of", &format!("{new:?}"))?;
                        report
                            .updated
                            .push((other.stem.clone(), format!("clone-of {old} -> {new}")));
                    }
                    Some(false) => {}
                    None => report.ambiguous.push((
                        other.stem.clone(),
                        format!("clone-of {old}: {} or {new_stem}?", clash.keep.stem),
                    )),
                }
            }
            let clones: Vec<String> = task::meta_in(&text, "clones")
                .map(task::parse_list)
                .unwrap_or_default();
            if clones.iter().any(|c| number(c) == Some(clash.id)) {
                let mine = other_id.is_some() && moving_clone_of == other_id;
                let theirs = other_id.is_some() && kept_clone_of == other_id;
                match ours(mine, theirs) {
                    Some(true) => {
                        let mut list: Vec<String> = clones
                            .into_iter()
                            .map(|c| {
                                if number(&c) == Some(clash.id) {
                                    new.clone()
                                } else {
                                    c
                                }
                            })
                            .collect();
                        list.sort();
                        list.dedup();
                        text = task::set_meta(&text, "clones", &format!("[{}]", list.join(",")))?;
                        report
                            .updated
                            .push((other.stem.clone(), format!("clones {old} -> {new}")));
                    }
                    Some(false) => {}
                    None => report.ambiguous.push((
                        other.stem.clone(),
                        format!("clones {old}: {} or {new_stem}?", clash.keep.stem),
                    )),
                }
            }

            // A typed link (ticket 207) is recorded on both tickets, each as
            // the inverse of the other, so each end vouches for the other.
            let entries = link_entries(&text);
            if entries.iter().any(|l| number(&l.id) == Some(clash.id)) {
                let vouches = |links: &[crate::links::Link]| {
                    entries
                        .iter()
                        .filter(|l| number(&l.id) == Some(clash.id))
                        .all(|l| {
                            links.iter().any(|back| {
                                back.kind == l.kind.inverse()
                                    && Some(back.id.as_str())
                                        == other.id.map(|n| format!("{n:03}")).as_deref()
                            })
                        })
                };
                match ours(vouches(&moving_links), vouches(&kept_links)) {
                    Some(true) => {
                        let mut list: Vec<crate::links::Link> = entries
                            .into_iter()
                            .map(|mut l| {
                                if number(&l.id) == Some(clash.id) {
                                    l.id = new.clone();
                                }
                                l
                            })
                            .collect();
                        list.sort();
                        list.dedup();
                        let written: Vec<String> = list.iter().map(|l| l.entry()).collect();
                        text = task::set_meta(&text, "links", &format!("[{}]", written.join(",")))?;
                        report
                            .updated
                            .push((other.stem.clone(), format!("links {old} -> {new}")));
                    }
                    Some(false) => {}
                    None => report.ambiguous.push((
                        other.stem.clone(),
                        format!("links {old}: {} or {new_stem}?", clash.keep.stem),
                    )),
                }
            }
        }

        if text != before {
            other.ensure_not_leased()?;
            fs::write(&other.path, text)
                .with_context(|| format!("writing {}", other.path.display()))?;
        }
    }
    Ok(())
}

/// The stems a ticket lists under `## Children`.
fn children(text: &str) -> Vec<String> {
    let Some((_, after)) = text.split_once("## Children") else {
        return Vec::new();
    };
    after
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| {
            let inner = line.trim().strip_prefix("- [[")?.strip_suffix("]]")?;
            Some(inner.to_string())
        })
        .collect()
}

/// A ticket's typed links (ticket 207), as written in `links`.
fn link_entries(text: &str) -> Vec<crate::links::Link> {
    task::meta_in(text, "links")
        .map(task::parse_list)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| crate::links::Link::parse(entry))
        .collect()
}

/// The numbers in one of a ticket's id lists (`related`, `clones`).
fn list_ids(text: &str, key: &str) -> Vec<u32> {
    task::meta_in(text, key)
        .map(task::parse_list)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| number(r))
        .collect()
}

/// `"042"` or `042` as a number.
fn number(value: &str) -> Option<u32> {
    value.trim().trim_matches('"').parse().ok()
}

fn read(path: &Path) -> Result<String> {
    crate::eol::read(path).with_context(|| format!("reading {}", path.display()))
}

/// How early `path` was first committed: the number of commits up to and
/// including the one that added it, so an earlier ticket always sorts first
/// (times can tie, commit order can't). Never-committed files sort last, so
/// a committed ticket keeps its number over a new one.
fn first_committed(repo: &Path, path: &Path) -> u64 {
    first_commit_depth(repo, path).unwrap_or(u64::MAX)
}

fn first_commit_depth(repo: &Path, path: &Path) -> Option<u64> {
    let git_out = |args: &[&str], path: Option<&Path>| -> Option<String> {
        let mut command = Command::new("git");
        command.arg("-C").arg(repo).args(args);
        if let Some(path) = path {
            command.arg(path);
        }
        let out = command.stderr(Stdio::null()).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    // Not `--follow`: tickets rendered from one template look like copies
    // of each other, and it traces them all back to the first ticket. A move
    // between columns keeps the file name, so the first commit adding that
    // name anywhere under the board is the ticket's own.
    let name = path.file_name()?.to_str()?;
    let spec = format!(":(glob)**/{name}");
    git_out(
        &["log", "--diff-filter=A", "--format=%H", "--", &spec],
        None,
    )
    .and_then(|log| log.lines().last().map(str::to_string))
    .and_then(|sha| git_out(&["rev-list", "--count", sha.trim()], None))
    .and_then(|count| count.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Column;

    fn board_at(dir: &Path) -> Board {
        let board = Board::at(dir.join("tasks"));
        board.create().unwrap();
        board
    }

    fn ticket(board: &Board, file: &str, front: &str, body: &str) {
        let id = &file[..3];
        fs::write(
            board.dir(Column::Backlog).join(file),
            format!("---\nid: \"{id}\"\ntitle: \"{file}\"\n{front}---\n\n{body}"),
        )
        .unwrap();
    }

    fn text(board: &Board, file: &str) -> String {
        crate::eol::read(board.dir(Column::Backlog).join(file)).unwrap()
    }

    #[test]
    fn the_second_ticket_moves_and_provable_references_follow_it() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        // 002 is held twice. 002-beta's child and its related ticket name
        // it by bare number; so does a ticket nobody's links account for.
        ticket(&board, "001-parent-of-nothing.md", "related: [002]\n", "");
        ticket(&board, "002-alpha.md", "created: \"2026-01-01\"\n", "");
        ticket(
            &board,
            "002-beta.md",
            "created: \"2026-02-01\"\nrelated: [004]\n",
            "## Children\n\n- [[003-child]]\n",
        );
        ticket(
            &board,
            "003-child.md",
            "parent: \"002\"\n",
            "see [[002-beta]]\n",
        );
        ticket(&board, "004-friend.md", "related: [002]\n", "");

        let clashes = plan(&board, None).unwrap();
        assert_eq!(clashes.len(), 1);
        assert_eq!(clashes[0].keep.stem, "002-alpha", "the older one keeps it");
        assert_eq!(clashes[0].moves[0].1, 5);

        let report = apply(&board, &clashes).unwrap();
        assert_eq!(report.renamed, [("002-beta".into(), "005-beta".into())]);
        assert!(text(&board, "005-beta.md").contains("id: \"005\""));
        let child = text(&board, "003-child.md");
        assert!(child.contains("parent: \"005\""), "{child}");
        assert!(child.contains("[[005-beta]]"), "{child}");
        assert!(text(&board, "004-friend.md").contains("related: [005]"));
        // Nothing says which 002 ticket 001 meant.
        assert!(text(&board, "001-parent-of-nothing.md").contains("related: [002]"));
        assert_eq!(report.ambiguous.len(), 1, "{:?}", report.ambiguous);
        assert!(report.ambiguous[0].0 == "001-parent-of-nothing");
        assert!(board.duplicate_ids().unwrap().is_empty());
    }

    #[test]
    fn clone_links_follow_the_ticket_they_can_prove_they_mean() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        // Two tickets hold 002. The later one has a copy (004) and is itself
        // a copy of 001; the older one has neither.
        ticket(&board, "001-source.md", "clones: [002]\n", "");
        ticket(&board, "002-alpha.md", "created: \"2026-01-01\"\n", "");
        ticket(
            &board,
            "002-beta.md",
            "created: \"2026-02-01\"\nclone-of: \"001\"\nclones: [004]\n",
            "",
        );
        ticket(&board, "004-copy.md", "clone-of: \"002\"\n", "");

        let clashes = plan(&board, None).unwrap();
        apply(&board, &clashes).unwrap();

        assert!(text(&board, "004-copy.md").contains("clone-of: \"005\""));
        assert!(text(&board, "005-beta.md").contains("clones: [004]"));
        // 001's bare 002 is the later ticket's: 005-beta says it is a copy of 001.
        assert!(text(&board, "001-source.md").contains("clones: [005]"));
        assert!(board.duplicate_ids().unwrap().is_empty());
    }

    #[test]
    fn typed_links_follow_the_ticket_they_can_prove_they_mean() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        // Two tickets hold 002. The later one requires 004, which says so back;
        // the older one links to nothing. 001 requires "002": the later one,
        // whose own links say it is required by 001.
        ticket(&board, "001-source.md", "links: [requires:002]\n", "");
        ticket(&board, "002-alpha.md", "created: \"2026-01-01\"\n", "");
        ticket(
            &board,
            "002-beta.md",
            "created: \"2026-02-01\"\nlinks: [required-by:001,requires:004]\n",
            "",
        );
        ticket(&board, "004-other.md", "links: [required-by:002]\n", "");

        let clashes = plan(&board, None).unwrap();
        apply(&board, &clashes).unwrap();

        assert!(text(&board, "004-other.md").contains("links: [required-by:005]"));
        assert!(text(&board, "001-source.md").contains("links: [requires:005]"));
        assert!(board.duplicate_ids().unwrap().is_empty());
    }

    #[test]
    fn keep_chooses_which_ticket_keeps_the_number() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        ticket(&board, "007-old.md", "created: \"2026-01-01\"\n", "");
        ticket(&board, "007-new.md", "created: \"2026-02-01\"\n", "");
        let clashes = plan(&board, Some("new")).unwrap();
        assert_eq!(clashes[0].keep.stem, "007-new");
        assert!(plan(&board, Some("nope")).is_err());
    }

    #[test]
    fn no_clash_plans_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        ticket(&board, "001-a.md", "", "");
        assert!(plan(&board, None).unwrap().is_empty());
    }
}
