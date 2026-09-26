//! The board: `<repo-root>/${AMD_DIR:-tasks}` with a directory per column.
//!
//! Status is the folder, so "moving a task" is a rename — `git mv` when the
//! file is tracked, so the history is the audit trail.

use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::git;
use crate::group::{GROUP_FILE, Group};
use crate::task::Task;
use crate::templates;

/// Default board directory name; override with `AMD_DIR`.
const DEFAULT_DIR: &str = "tasks";

/// Where archived tickets go. Not a column: it's off the board, and its
/// contents are gitignored, so archiving takes a ticket out of the history
/// rather than recording another status.
const ARCHIVE: &str = "archive";

/// Keeps the drawer itself in git while ignoring what's in it.
const ARCHIVE_GITIGNORE: &str = "*\n!.gitignore\n";

/// The verb that takes a ticket off the board (`amd abandon <ref>`). Reserved
/// the same way a column name is: an epic or sprint called `abandon` would
/// make the destination-verb dispatch ambiguous the moment it existed.
pub const ABANDON: &str = "abandon";

/// The board's id counter: the number the next ticket will take. One line,
/// tracked with the board, so ids keep climbing even when tickets are deleted
/// outright rather than archived — a scan can only see what's still there.
const COUNTER: &str = ".next-id";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column {
    Backlog,
    Todo,
    Doing,
    Done,
}

impl Column {
    pub const ALL: [Column; 4] = [Column::Backlog, Column::Todo, Column::Doing, Column::Done];

    pub fn as_str(self) -> &'static str {
        match self {
            Column::Backlog => "backlog",
            Column::Todo => "todo",
            Column::Doing => "doing",
            Column::Done => "done",
        }
    }

    pub fn parse(name: &str) -> Option<Column> {
        Column::ALL
            .into_iter()
            .find(|column| column.as_str() == name.to_ascii_lowercase())
    }
}

impl fmt::Display for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A move was refused because its destination column is already at (or
/// past) its configured WIP limit. Gates entry only — a column that's
/// already over its limit stays movable-out-of, since this never looks at
/// where a ticket is leaving from.
#[derive(Debug)]
pub struct WipLimitExceeded {
    pub column: Column,
    pub limit: u32,
    pub current: usize,
}

impl fmt::Display for WipLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/ already has {} of {} allowed — move something out first, or use --force",
            self.column, self.current, self.limit
        )
    }
}

impl std::error::Error for WipLimitExceeded {}

/// What `Board::empty_group` actually did — every ticket it moved, and
/// every ticket (stem, reason) it skipped rather than aborting the batch
/// over.
pub struct EmptiedGroup {
    pub moved: Vec<PathBuf>,
    pub skipped: Vec<(String, String)>,
}

pub struct Board {
    /// Absolute path to the board directory itself (`…/tasks`).
    pub root: PathBuf,
}

impl Board {
    /// A board at an explicit path, for tests and for a caller handed one
    /// rather than discovering it.
    pub fn at(root: PathBuf) -> Board {
        Board { root }
    }

    /// The board inside a repository whose root we already know — a registered
    /// one, rather than the repository we happen to be standing in.
    pub fn in_repo(repo: &std::path::Path) -> Board {
        Board {
            root: repo.join(dir_name()),
        }
    }
}

impl Board {
    /// Where the board *would* live for the current directory. Does not
    /// require it to exist — `amd init` needs the path before the directory.
    pub fn locate() -> Result<Board> {
        let dir = dir_name();
        let repo = git::repo_root().ok_or_else(|| {
            anyhow!("not inside a git repository (agile-md boards live at <repo>/{dir})")
        })?;
        Ok(Board {
            root: repo.join(dir),
        })
    }

    /// Open an existing board, or say where it should have been. Creating one
    /// is a decision for the caller — the CLI asks, the GUI offers a button.
    pub fn open() -> Result<Board> {
        let board = Board::locate()?;
        if board.root.is_dir() {
            return Ok(board);
        }
        bail!(
            "no board at {} — run 'amd init' (or set AMD_YES=1)",
            board.root.display()
        )
    }

    /// `locate()`, and — only when that fails because the current directory
    /// isn't inside a git repository at all — a fallback for the layout
    /// `amd init <project>` now creates: a plain parent directory holding
    /// `{project}-tickets` (and optionally `{project}-documents`) as
    /// siblings. Lets `amd`/`amdui` be run from that parent directory
    /// itself, not only from one `cd` deeper inside the tickets repository.
    ///
    /// Not tied to the `-tickets` naming convention specifically — any
    /// immediate child that is both a git repository and has a board of its
    /// own qualifies, the same `has_board()` test the registry already uses
    /// elsewhere. Two or more qualifying children is an error naming all of
    /// them, never a silent guess, the same principle `find` already applies
    /// to an ambiguous ticket reference.
    pub fn discover() -> Result<Board> {
        if let Ok(board) = Board::locate() {
            return Ok(board);
        }
        let cwd = env::current_dir().context("reading the current directory")?;
        match project_siblings(&cwd).as_slice() {
            [only] => Ok(Board::in_repo(only)),
            [] => bail!(
                "not inside a git repository, and no project board among this directory's own \
                 subdirectories either (agile-md boards live at <repo>/{})",
                dir_name()
            ),
            many => {
                let names: Vec<String> = many
                    .iter()
                    .filter_map(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .collect();
                bail!(
                    "more than one project board here ({}); cd into one directly",
                    names.join(", ")
                )
            }
        }
    }

    /// The board directory's name (`tasks`), for display and templates.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(dir_name)
    }

    pub fn dir(&self, column: Column) -> PathBuf {
        self.root.join(column.as_str())
    }

    /// The repository containing the board — where `README.md` and the
    /// board-local settings file live, as siblings of the board directory
    /// rather than inside it.
    pub fn repo_root(&self) -> &Path {
        self.root.parent().unwrap_or(&self.root)
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.root.join("templates")
    }

    pub fn archive_dir(&self) -> PathBuf {
        self.root.join(ARCHIVE)
    }

    /// Where the counter lived before it moved into the board-local settings
    /// file's `next-id` key. Still read, as a fallback, for boards that
    /// predate the move — never written to again.
    pub fn legacy_counter_path(&self) -> PathBuf {
        self.root.join(COUNTER)
    }

    /// What the next id should be, if something on disk says so — the
    /// board-local settings file first. A board that predates the move to
    /// board-local settings still has the legacy `.next-id` file instead;
    /// the first time that's seen, its value is migrated into settings and
    /// the legacy file is removed, so this only ever happens once per board
    /// rather than reading around the old file forever.
    fn stored_next_id(&self) -> Option<u32> {
        let settings = crate::board_settings::load(self);
        if let Some(id) = settings.next_id() {
            return Some(id);
        }
        let legacy = self.legacy_next_id()?;
        let mut settings = settings;
        settings.set_next_id(legacy);
        if settings.save(self).is_ok() {
            let _ = fs::remove_file(self.legacy_counter_path());
        }
        Some(legacy)
    }

    fn legacy_next_id(&self) -> Option<u32> {
        fs::read_to_string(self.legacy_counter_path())
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()
    }

    /// Scaffold the columns, the board-local settings file's counter, and
    /// the repository's README. Idempotent.
    pub fn create(&self) -> Result<()> {
        for column in Column::ALL {
            let dir = self.dir(column);
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            let keep = dir.join(".gitkeep");
            if !keep.exists() {
                fs::write(&keep, "").with_context(|| format!("creating {}", keep.display()))?;
            }
        }
        // The drawer is tracked; what you put in it is not.
        let archive = self.archive_dir();
        fs::create_dir_all(&archive).with_context(|| format!("creating {}", archive.display()))?;
        let ignore = archive.join(".gitignore");
        if !ignore.exists() {
            fs::write(&ignore, ARCHIVE_GITIGNORE)
                .with_context(|| format!("creating {}", ignore.display()))?;
        }

        if self.stored_next_id().is_none() {
            let mut settings = crate::board_settings::load(self);
            settings.set_next_id(1);
            settings.save(self)?;
        }

        let readme = self.repo_root().join("README.md");
        if !readme.exists() {
            let rendered = templates::render_board_readme(self)?;
            fs::write(&readme, rendered)
                .with_context(|| format!("writing {}", readme.display()))?;
        }
        Ok(())
    }

    /// Tasks in one column, ordered by id then filename.
    /// Every ticket in a column, epic folders included.
    ///
    /// The backlog may hold subdirectories — one per epic or sprint — and their
    /// tickets are as real as the loose ones. Anything that counts ids must see
    /// them: a ticket invisible to the scan is an id waiting to be handed out
    /// twice, which silently repoints every reference to it.
    pub fn tasks_in(&self, column: Column) -> Result<Vec<Task>> {
        let dir = self.dir(column);
        let mut tasks = self.tasks_directly_in(&dir, column, None)?;
        for epic in self.epics_in(column)? {
            tasks.extend(self.tasks_directly_in(&dir.join(&epic), column, Some(epic))?);
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.stem.cmp(&b.stem)));
        Ok(tasks)
    }

    /// The epic folders of a column, named by directory. Only the backlog has
    /// them: an epic is a way to group work you have not committed to yet, and
    /// letting them appear in every column would make "status is the folder"
    /// ambiguous.
    pub fn epics_in(&self, column: Column) -> Result<Vec<String>> {
        if column != Column::Backlog {
            return Ok(Vec::new());
        }
        let dir = self.dir(column);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
        };
        let mut epics = Vec::new();
        for entry in entries {
            let path = entry
                .with_context(|| format!("reading {}", dir.display()))?
                .path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|name| name.to_str())
                && !name.starts_with('.')
            {
                epics.push(name.to_string());
            }
        }
        epics.sort();
        Ok(epics)
    }

    fn tasks_directly_in(
        &self,
        dir: &Path,
        column: Column,
        epic: Option<String>,
    ) -> Result<Vec<Task>> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
        };
        let mut tasks = Vec::new();
        for entry in entries {
            let path = entry
                .with_context(|| format!("reading {}", dir.display()))?
                .path();
            if path.file_name().and_then(|n| n.to_str()) == Some(GROUP_FILE) {
                continue;
            }
            if path.is_file()
                && let Some(mut task) = Task::from_path(&path, column)
            {
                task.epic = epic.clone();
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    /// Every group in the backlog — epics and sprints alike — read from their
    /// own directories.
    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut groups = Vec::new();
        for name in self.epics_in(Column::Backlog)? {
            groups.push(Group::read(&self.dir(Column::Backlog).join(name))?);
        }
        Ok(groups)
    }

    /// Look one up by directory name.
    pub fn group(&self, name: &str) -> Result<Group> {
        let dir = self.dir(Column::Backlog).join(name);
        if !dir.is_dir() {
            bail!("no group called {name}");
        }
        Group::read(&dir)
    }

    /// Create an epic or a sprint. The directory name is the identity, so a
    /// second group of the same name is a mistake rather than a merge.
    pub fn create_group(&self, group: &Group) -> Result<()> {
        if group.name.trim().is_empty() {
            bail!("a group needs a name");
        }
        // The destination-verb dispatch (`amd <name> <ref>`) is checked
        // against columns, `abandon`, and epics in that order — a group
        // sharing a name with the first two would be silently unreachable.
        if Column::parse(&group.name).is_some() || group.name.eq_ignore_ascii_case(ABANDON) {
            bail!(
                "'{}' collides with the destination verb of the same name — pick another name",
                group.name
            );
        }
        if group.dir.exists() {
            bail!("{} already exists", group.name);
        }
        group.save()
    }

    /// File a ticket under an epic, or move it back to the loose backlog with
    /// `None`. Inside `backlog/` the folder is the move, and the frontmatter
    /// is updated to match; outside it (`todo`/`doing`/`done`) there is no
    /// group directory to move into, so this only ever writes the `epic`
    /// frontmatter key in place — the ticket stays exactly where it is,
    /// column included. Calling this on a `doing`/`done` ticket does not
    /// send it back to the backlog.
    pub fn set_epic(&self, task: &Task, epic: Option<&str>) -> Result<PathBuf> {
        // Checked up front, not left to set_epic_meta's own write_checked
        // afterwards — that would risk a half-moved, half-refused ticket if
        // someone else holds the lease.
        task.ensure_not_leased()?;
        // A started sprint takes tickets in and lets them out. Teams do this —
        // more often when they are new — and it is poor practice that skews the
        // charts, but a tool that refuses leaves people editing frontmatter by
        // hand, which skews the charts *and* loses the record. Every move is a
        // `git mv`, so the scope change is dated in the history and a burnup
        // can show it (docs/adr/0009-sprint-scope-and-archiving.md).
        if let Some(name) = epic {
            let group = self.group(name)?;
            // An unsized ticket in a sprint makes the sprint's total a lie,
            // which is the one number a sprint is for. This rule stays.
            if group.is_sprint() && task.points().is_none() {
                bail!("{} needs points before it can go in {name}", task.stem);
            }
        }

        if task.column != Column::Backlog {
            let hash = task.hash()?;
            task.set_epic_meta(hash, epic.unwrap_or_default())?;
            return Ok(task.path.clone());
        }

        let dir = match epic {
            Some(name) => self.dir(Column::Backlog).join(name),
            None => self.dir(Column::Backlog),
        };
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let dest = dir.join(task.file_name());
        if dest != task.path {
            if dest.exists() {
                bail!("{} already exists", dest.display());
            }
            // Same rule as move_task: `git mv` so history follows the rename
            // when the ticket is tracked, a plain rename when it is not.
            if git::is_tracked(&self.root, &task.path) {
                git::mv(&self.root, &task.path, &dest)?;
            } else {
                fs::rename(&task.path, &dest)
                    .with_context(|| format!("moving {}", task.path.display()))?;
            }
        }
        let moved = Task::from_path(&dest, Column::Backlog)
            .ok_or_else(|| anyhow::anyhow!("{} is not a task", dest.display()))?;
        let hash = moved.hash()?;
        moved.set_epic_meta(hash, epic.unwrap_or_default())?;
        Ok(dest)
    }

    /// Empty a group's folder into the loose backlog at once — every
    /// ticket currently in it, no precondition, the same "no precondition"
    /// as an epic already has for a single ticket. Reuses `set_epic` per
    /// ticket rather than a new mechanism, so it inherits the same
    /// lease/hash checks that setter already has. A ticket that can't move
    /// (someone else's active lease, say) is skipped rather than aborting
    /// the whole batch — reported the same way `amd import` already
    /// reports a skipped ticket.
    pub fn empty_group(&self, name: &str) -> Result<EmptiedGroup> {
        let mut moved = Vec::new();
        let mut skipped = Vec::new();
        for task in self.tasks_in_group(name)? {
            let stem = task.stem.clone();
            match self.set_epic(&task, None) {
                Ok(dest) => moved.push(dest),
                Err(err) => skipped.push((stem, err.to_string())),
            }
        }
        Ok(EmptiedGroup { moved, skipped })
    }

    /// Delete a group outright — its `_group.md` and the (now-empty)
    /// directory. Refuses if it still has any tickets, checked the same way
    /// `empty_group` finds them: every column, not just the physical
    /// folder, since a committed sprint's tickets can sit in `doing`/`done`
    /// while still claiming membership. `empty` it first, same two-step
    /// `create_group`'s own "already exists" refusal expects the caller to
    /// resolve deliberately rather than the tool guessing.
    pub fn delete_group(&self, name: &str) -> Result<()> {
        let group = self.group(name)?;
        let tickets = self.tasks_in_group(name)?;
        if !tickets.is_empty() {
            bail!(
                "{name} still has {} ticket(s) — empty it first",
                tickets.len()
            );
        }
        fs::remove_dir_all(&group.dir).with_context(|| format!("removing {}", group.dir.display()))
    }

    /// Every task on the board, column order first.
    /// Every ticket belonging to a group, in whatever column it has reached.
    ///
    /// A sprint's scope does not shrink because somebody started work. Counting
    /// only `backlog/` made a sprint's points fall as tickets moved to
    /// `doing/`, which is the opposite of what that number is for.
    ///
    /// Which field answers "what group is this in" depends on where the ticket
    /// is, and the two are not interchangeable. Inside `backlog/` the **folder**
    /// decides — a ticket dragged into an epic directory by hand is in that
    /// epic, frontmatter or no. Outside it there are no group directories, so
    /// the `epic` key written by `set_epic` is the only carrier.
    ///
    /// The archive is deliberately not searched: an archived ticket has left
    /// the board.
    pub fn tasks_in_group(&self, name: &str) -> Result<Vec<Task>> {
        Ok(self
            .tasks()?
            .into_iter()
            .filter(|task| match task.column {
                Column::Backlog => task.epic.as_deref() == Some(name),
                _ => task.epic_meta().as_deref() == Some(name),
            })
            .collect())
    }

    pub fn tasks(&self) -> Result<Vec<Task>> {
        let mut all = Vec::new();
        for column in Column::ALL {
            all.extend(self.tasks_in(column)?);
        }
        Ok(all)
    }

    /// Every tag already in use on the board, sorted and deduplicated — the
    /// suggestions offered when a task is created interactively.
    pub fn tags(&self) -> Result<Vec<String>> {
        let mut tags: Vec<String> = Vec::new();
        for task in self.tasks()? {
            let Some(raw) = task.meta("tags") else {
                continue;
            };
            for tag in raw.trim_matches(['[', ']']).split(',') {
                let tag = tag.trim().trim_matches(['"', '\'']).to_string();
                if !tag.is_empty() && !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        tags.sort();
        Ok(tags)
    }

    /// Every ticket carrying `tag`, across every column, in board order.
    /// `amd import`'s `DUPLICATE` tag is the motivating case — reviewing
    /// what an import flagged — but this filters on any tag, not just that
    /// one.
    pub fn tasks_tagged(&self, tag: &str) -> Result<Vec<Task>> {
        Ok(self
            .tasks()?
            .into_iter()
            .filter(|task| task.has_tag(tag))
            .collect())
    }

    /// Everyone with a ticket here, sorted — the suggestions when assigning
    /// one.
    pub fn assignees(&self) -> Result<Vec<String>> {
        let mut assignees: Vec<String> = Vec::new();
        for task in self.tasks()? {
            if let Some(who) = task.assignee()
                && !assignees.contains(&who)
            {
                assignees.push(who);
            }
        }
        assignees.sort();
        Ok(assignees)
    }

    /// Every task's `NNN-slug`, the strings `amd new` completes a related ref
    /// against.
    pub fn stems(&self) -> Result<Vec<String>> {
        Ok(self.tasks()?.into_iter().map(|task| task.stem).collect())
    }

    /// Tickets that have been archived. Not part of the board, but they still
    /// hold ids.
    pub fn archived(&self) -> Result<Vec<Task>> {
        let dir = self.archive_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
        };
        let mut tasks = Vec::new();
        for entry in entries {
            let path = entry
                .with_context(|| format!("reading {}", dir.display()))?
                .path();
            // Archived tickets keep the column they were last in; nothing reads
            // it, but it means a restored file lands somewhere sensible.
            if path.is_file()
                && let Some(task) = Task::from_path(&path, Column::Todo)
            {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    /// The id the next ticket will take: whichever is higher of the counter
    /// file and one past the highest id on the board.
    ///
    /// The counter is the authority — it remembers ids whose tickets have gone,
    /// archived or deleted, which no scan can. The scan is the safety net: a
    /// lost, stale or badly merged counter can then never hand out an id that
    /// is already in use, and a ticket added by hand still pushes it along.
    /// Only the columns are scanned; the archive is off the board and the
    /// counter already accounts for it.
    pub fn next_id(&self) -> Result<u32> {
        let highest = self
            .tasks()?
            .iter()
            .filter_map(|task| task.id)
            .max()
            .unwrap_or(0);
        Ok(self.stored_next_id().unwrap_or(0).max(highest + 1))
    }

    /// Record that `used` has been taken, so the next ticket gets the one
    /// after. Called once the ticket is on disk, never when one is merely
    /// drafted — an abandoned draft should leave no gap.
    pub fn record_id(&self, used: u32) -> Result<()> {
        let next = used.saturating_add(1);
        if self.stored_next_id().is_some_and(|stored| stored >= next) {
            return Ok(());
        }
        let mut settings = crate::board_settings::load(self);
        settings.set_next_id(next);
        settings.save(self)
    }

    /// Take a ticket off the board. The archive is gitignored, so a
    /// tracked ticket leaves the index (`git rm --cached`) and then moves —
    /// `git mv` would refuse the ignored destination, and forcing it would put
    /// the ticket back into the history the .gitignore is there to keep it out of.
    ///
    /// Also cleans up the ticket's branch, if it has one that's empty: a
    /// ticket can sit in `todo/` — where entering it creates a branch — and be
    /// abandoned before real work starts, so moving the trigger earlier just
    /// relocates which column produces unused branches (docs/adr/0006). The
    /// returned name is `Some` when a branch was actually deleted.
    pub fn archive(&self, task: &Task) -> Result<(PathBuf, Option<String>)> {
        task.ensure_not_leased()?;
        // Moving a ticket out of a started sprint leaves a `git mv` in the
        // history: the scope change is dated and the points are still readable
        // on disk. Archiving from inside one leaves neither — the file is
        // gitignored and untracked, so the board no longer shows that those
        // points were ever committed to. Take it out of the sprint first; then
        // archive it. Two steps, and the record survives the first.
        if let Some(name) = task.epic.as_deref()
            && let Ok(group) = self.group(name)
            && group.is_sprint()
            && !group.accepts_changes()
        {
            bail!(
                "{} is in {name}, which has started — move it out of the sprint before archiving it",
                task.stem
            );
        }
        let dir = self.archive_dir();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let dest = dir.join(task.file_name());
        if dest.exists() {
            bail!("{} already exists", dest.display());
        }
        // Read before the move, not after: once the file lands in dest, it no
        // longer exists at task.path for task.branch() to read.
        let branch = task.branch();
        if git::is_tracked(&self.root, &task.path) {
            git::untrack(&self.root, &task.path)?;
        }
        fs::rename(&task.path, &dest).with_context(|| format!("moving {}", task.path.display()))?;
        let deleted = branch.and_then(|name| self.cleanup_branch(&name));
        Ok((dest, deleted))
    }

    /// Delete a branch if it's safe to: it exists locally, isn't checked out
    /// right now, has never been pushed, and carries no commits `HEAD`
    /// doesn't already have. Anything else — real work on it, shared with a
    /// remote — is left alone. Best effort: a git failure here doesn't fail
    /// the archive, which has already happened.
    fn cleanup_branch(&self, name: &str) -> Option<String> {
        if name.is_empty()
            || !git::branch_exists(&self.root, name)
            || git::current_branch(&self.root).as_deref() == Some(name)
            || git::branch_pushed(&self.root, name)
            || git::commits_ahead(&self.root, name) != Some(0)
        {
            return None;
        }
        git::delete_branch(&self.root, name).ok()?;
        Some(name.to_string())
    }

    /// Resolve a task reference: a numeric id (`7`, `007`) or a unique slug
    /// substring. Ambiguity is an error, never a guess.
    pub fn find(&self, reference: &str) -> Result<Task> {
        let tasks = self.tasks()?;
        let mut hits: Vec<Task> = if let Ok(id) = reference.parse::<u32>() {
            tasks.into_iter().filter(|t| t.id == Some(id)).collect()
        } else {
            tasks
                .into_iter()
                .filter(|t| t.stem.contains(reference))
                .collect()
        };
        match hits.len() {
            0 => bail!("no task matches '{reference}'"),
            1 => Ok(hits.remove(0)),
            n => {
                let names: Vec<&str> = hits.iter().map(|t| t.stem.as_str()).collect();
                bail!(
                    "'{reference}' matches {n} tasks; be more specific ({})",
                    names.join(", ")
                )
            }
        }
    }

    /// Move a task to another column, preferring `git mv` so the rename is
    /// tracked. Deliberately does not commit — that stays the user's call.
    ///
    /// Returns where the ticket landed. It deliberately says nothing: a window
    /// has nowhere to put a line of text, and a library that prints is a
    /// library that has decided what its caller's output looks like
    /// (docs/adr/0004-one-library-two-interfaces.md).
    pub fn move_task(&self, task: &Task, to: Column, force: bool) -> Result<PathBuf> {
        if task.column == to {
            bail!("already in {to}/");
        }
        task.ensure_not_leased()?;
        if !force && let Some(limit) = crate::board_settings::load(self).wip_limit(to) {
            let current = self.tasks_in(to)?.len();
            if current as u32 >= limit {
                return Err(WipLimitExceeded {
                    column: to,
                    limit,
                    current,
                }
                .into());
            }
        }
        let dir = self.dir(to);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let dest = dir.join(task.file_name());
        if dest.exists() {
            bail!("{} already exists", dest.display());
        }
        if git::is_tracked(&self.root, &task.path) {
            git::mv(&self.root, &task.path, &dest)?;
        } else {
            fs::rename(&task.path, &dest)
                .with_context(|| format!("moving {}", task.path.display()))?;
        }
        Ok(dest)
    }
}

fn dir_name() -> String {
    env::var("AMD_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
        .unwrap_or_else(|| DEFAULT_DIR.to_string())
}

/// Every immediate child of `dir` that is both a git repository and has a
/// board of its own — the candidates `Board::discover()` chooses from.
/// Separated out so it can be tested against a real temporary directory
/// without touching the process's current directory, which `discover()`
/// itself reads and which isn't safe to mutate with tests running in
/// parallel (`registry.rs`'s `remember_in` uses the same split, for the
/// same reason). A directory that can't be read (removed mid-scan, no
/// permission) contributes no candidates rather than failing the whole
/// scan — the same "best effort" stance the registry takes elsewhere.
fn project_siblings(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.join(".git").is_dir() && path.join(dir_name()).is_dir())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_at(dir: &std::path::Path) -> Board {
        let board = Board::at(dir.join("tasks"));
        board.create().expect("board");
        board
    }

    fn touch(board: &Board, name: &str) {
        fs::write(board.dir(Column::Todo).join(name), "---\n---\n").unwrap();
    }

    fn backlog_ticket(board: &Board, name: &str) {
        let body = format!("---\nid: \"{}\"\ntitle: \"T\"\n---\n\nbody\n", &name[..3]);
        fs::write(board.dir(Column::Backlog).join(name), body).unwrap();
    }

    fn backlog_ticket_with_branch(board: &Board, name: &str, branch: &str) {
        let body = format!(
            "---\nid: \"{}\"\ntitle: \"T\"\nbranch-name: \"{branch}\"\n---\n\nbody\n",
            &name[..3]
        );
        fs::write(board.dir(Column::Backlog).join(name), body).unwrap();
    }

    /// A real git repository, seeded with an initial commit so `HEAD` exists
    /// — the branch-cleanup checks need real history to compare against, not
    /// just a board directory on disk.
    fn git_repo_board(dir: &std::path::Path) -> Board {
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.co"]);
        git(&["config", "user.name", "t"]);
        let board = board_at(dir);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "seed"]);
        board
    }

    #[test]
    fn archiving_deletes_a_branch_with_no_commits() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        backlog_ticket_with_branch(&board, "001-thing.md", "feature/thing");
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["branch", "feature/thing"])
            .status()
            .unwrap();

        let task = board.find("001").unwrap();
        let (_, deleted) = board.archive(&task).unwrap();
        assert_eq!(deleted.as_deref(), Some("feature/thing"));
        assert!(!git::branch_exists(&board.root, "feature/thing"));
    }

    #[test]
    fn archiving_leaves_a_branch_with_real_commits_alone() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        backlog_ticket_with_branch(&board, "001-thing.md", "feature/thing");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap();
        };
        // Commit the ticket to the base branch first: an uncommitted file
        // would vanish from the working tree the moment `switch` back finds
        // it missing from that commit's own tree.
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "add ticket"]);
        git(&["switch", "-c", "feature/thing"]);
        fs::write(dir.path().join("work.txt"), "did something\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "real work"]);
        git(&["switch", "-"]); // back to whichever branch was checked out before

        let task = board.find("001").unwrap();
        let (_, deleted) = board.archive(&task).unwrap();
        assert_eq!(deleted, None, "a branch with real commits must survive");
        assert!(git::branch_exists(&board.root, "feature/thing"));
    }

    #[test]
    fn archiving_leaves_the_currently_checked_out_branch_alone() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        backlog_ticket_with_branch(&board, "001-thing.md", "feature/thing");
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["switch", "-c", "feature/thing"])
            .status()
            .unwrap();

        let task = board.find("001").unwrap();
        let (_, deleted) = board.archive(&task).unwrap();
        assert_eq!(deleted, None, "can't delete the branch you're standing on");
        assert!(git::branch_exists(&board.root, "feature/thing"));
    }

    #[test]
    fn archiving_leaves_a_pushed_branch_alone() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        backlog_ticket_with_branch(&board, "001-thing.md", "feature/thing");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap();
        };
        git(&["branch", "feature/thing"]);
        // No real remote needed: a matching remote-tracking ref is what
        // `branch_pushed` actually checks for.
        git(&["update-ref", "refs/remotes/origin/feature/thing", "HEAD"]);

        let task = board.find("001").unwrap();
        let (_, deleted) = board.archive(&task).unwrap();
        assert_eq!(deleted, None, "a pushed branch might be in use elsewhere");
        assert!(git::branch_exists(&board.root, "feature/thing"));
    }

    #[test]
    fn archiving_a_ticket_with_no_branch_does_nothing_extra() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        backlog_ticket(&board, "001-loose.md");

        let task = board.find("001").unwrap();
        let (_, deleted) = board.archive(&task).unwrap();
        assert_eq!(deleted, None);
    }

    #[test]
    fn tickets_inside_an_epic_folder_are_still_on_the_board() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        backlog_ticket(&board, "001-loose.md");
        let epic = board.dir(Column::Backlog).join("checkout");
        fs::create_dir_all(&epic).unwrap();
        fs::write(epic.join("002-filed.md"), "---\nid: \"002\"\n---\n").unwrap();

        let tasks = board.tasks_in(Column::Backlog).unwrap();
        assert_eq!(tasks.len(), 2, "the filed ticket must be seen");
        let filed = tasks.iter().find(|t| t.stem == "002-filed").unwrap();
        assert_eq!(filed.epic.as_deref(), Some("checkout"));

        // The reason this matters: an unseen ticket is an id handed out twice.
        assert_eq!(board.next_id().unwrap(), 3);
        assert_eq!(board.epics_in(Column::Backlog).unwrap(), ["checkout"]);
        // Epics are a backlog idea only.
        assert!(board.epics_in(Column::Todo).unwrap().is_empty());
    }

    fn sprint(board: &Board, name: &str) -> crate::group::Group {
        let group = crate::group::Group {
            dir: board.dir(Column::Backlog).join(name),
            name: name.to_string(),
            kind: crate::group::Kind::Sprint,
            description: String::new(),
            days: crate::group::DEFAULT_DAYS,
            state: crate::group::State::Pending,
        };
        board.create_group(&group).unwrap();
        group
    }

    #[test]
    fn a_sprint_refuses_an_unsized_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        sprint(&board, "sprint-1");
        backlog_ticket(&board, "001-unsized.md");

        let task = board.find("001").unwrap();
        let err = board.set_epic(&task, Some("sprint-1")).unwrap_err();
        assert!(
            err.to_string().contains("needs points"),
            "unexpected: {err}"
        );
        assert!(task.path.exists(), "the ticket must not have moved");

        // Sized, it goes in.
        task.set_points(task.hash().unwrap(), "3").unwrap();
        board.set_epic(&task, Some("sprint-1")).unwrap();
        assert_eq!(board.find("001").unwrap().epic.as_deref(), Some("sprint-1"));
    }

    #[test]
    fn an_epic_takes_unsized_tickets() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        backlog_ticket(&board, "001-unsized.md");
        let task = board.find("001").unwrap();
        // Sizing is a sprint's requirement, not an epic's: an epic is where
        // work goes before anyone has estimated it.
        board.set_epic(&task, Some("checkout")).unwrap();
        assert_eq!(board.find("001").unwrap().epic.as_deref(), Some("checkout"));
    }

    #[test]
    fn a_started_sprint_takes_tickets_in_and_out_but_refuses_an_archive() {
        // ADR-0009. The old rule — a started sprint is immutable — was one
        // `amd abandon` wide, and refusing only pushed the change out of the
        // tool's sight. What it defends now is the *record*: a move out is a
        // git mv, an archive is gitignored and is not.
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        let mut group = sprint(&board, "sprint-1");
        backlog_ticket(&board, "001-in.md");
        backlog_ticket(&board, "002-out.md");

        let inside = board.find("001").unwrap();
        inside.set_points(inside.hash().unwrap(), "5").unwrap();
        board.set_epic(&inside, Some("sprint-1")).unwrap();
        group.start().unwrap();

        // Tickets go in after the start...
        let outside = board.find("002").unwrap();
        outside.set_points(outside.hash().unwrap(), "2").unwrap();
        board
            .set_epic(&outside, Some("sprint-1"))
            .expect("a started sprint takes a sized ticket");

        // ...and come back out again.
        let inside = board.find("001").unwrap();
        board
            .set_epic(&inside, None)
            .expect("a started sprint lets a ticket out");

        // But not straight into the archive.
        let still_in = board.find("002").unwrap();
        let err = board.archive(&still_in).unwrap_err();
        assert!(
            err.to_string().contains("before archiving"),
            "unexpected: {err}"
        );

        // Out of the sprint first, and then it may go.
        let out = board.find("002").unwrap();
        board.set_epic(&out, None).unwrap();
        let out = board.find("002").unwrap();
        board
            .archive(&out)
            .expect("archivable once out of the sprint");
    }

    #[test]
    fn a_sprint_keeps_its_tickets_when_they_leave_the_backlog() {
        // The bug: scope was counted from backlog/ alone, so a sprint's points
        // fell as work started on them.
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        sprint(&board, "sprint-1");
        backlog_ticket(&board, "001-in.md");

        let task = board.find("001").unwrap();
        task.set_points(task.hash().unwrap(), "5").unwrap();
        board.set_epic(&task, Some("sprint-1")).unwrap();
        assert_eq!(board.tasks_in_group("sprint-1").unwrap().len(), 1);

        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Doing, false).unwrap();
        assert_eq!(
            board.tasks_in_group("sprint-1").unwrap().len(),
            1,
            "a ticket in doing/ still belongs to its sprint"
        );
    }

    #[test]
    fn set_epic_tags_a_non_backlog_ticket_in_place_without_moving_it() {
        // Before this, set_epic's directory-move logic ran unconditionally,
        // so filing a doing/ ticket into an epic would have silently
        // relocated it back into backlog/<epic>/ — nothing called it on a
        // non-backlog task before the GUI's membership checkboxes needed to.
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        backlog_ticket(&board, "001-a.md");
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Doing, false).unwrap();
        let path_before = board.find("001").unwrap().path;

        let task = board.find("001").unwrap();
        board.set_epic(&task, Some("checkout")).unwrap();

        let after = board.find("001").unwrap();
        assert_eq!(after.column, Column::Doing, "must not move column");
        assert_eq!(after.path, path_before, "must not move the file");
        assert_eq!(after.epic_meta().as_deref(), Some("checkout"));
        assert_eq!(
            board.tasks_in_group("checkout").unwrap().len(),
            1,
            "membership is visible via tasks_in_group despite no directory move"
        );

        // And clearing it is likewise metadata-only.
        let task = board.find("001").unwrap();
        board.set_epic(&task, None).unwrap();
        let after = board.find("001").unwrap();
        assert_eq!(after.column, Column::Doing);
        assert_eq!(after.path, path_before);
        assert_eq!(after.epic_meta(), None);
    }

    #[test]
    fn the_group_file_is_not_a_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        sprint(&board, "sprint-1");
        let tasks = board.tasks_in(Column::Backlog).unwrap();
        assert!(tasks.is_empty(), "_group.md must not count as a ticket");
        assert_eq!(board.next_id().unwrap(), 1, "nor consume an id");
    }

    #[test]
    fn a_group_cannot_be_named_after_a_column_or_abandon() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        for name in ["todo", "DONE", "Abandon"] {
            let group = crate::group::Group {
                dir: board.dir(Column::Backlog).join(name),
                name: name.to_string(),
                kind: crate::group::Kind::Epic,
                description: String::new(),
                days: crate::group::DEFAULT_DAYS,
                state: crate::group::State::Pending,
            };
            let err = board.create_group(&group).unwrap_err();
            assert!(err.to_string().contains("collides"), "{name}: {err}");
            assert!(!group.dir.exists(), "{name}: must not have been created");
        }
    }

    #[test]
    fn filing_under_an_epic_moves_the_file_and_updates_the_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        backlog_ticket(&board, "001-loose.md");
        // The group has to exist first: filing onto a name that isn't there
        // would turn a typo into a new epic.
        for name in ["checkout", "payments"] {
            fs::create_dir_all(board.dir(Column::Backlog).join(name)).unwrap();
        }
        let task = board.find("001").unwrap();

        let dest = board.set_epic(&task, Some("checkout")).unwrap();
        assert!(dest.ends_with("backlog/checkout/001-loose.md"), "{dest:?}");
        assert!(!task.path.exists(), "the original must have moved");
        let filed = board.find("001").unwrap();
        assert_eq!(filed.epic_meta().as_deref(), Some("checkout"));
        assert!(
            fs::read_to_string(&filed.path).unwrap().ends_with("body\n"),
            "the body must survive the move"
        );

        // Moving to another epic rewrites the key rather than accumulating.
        board.set_epic(&filed, Some("payments")).unwrap();
        let moved = board.find("001").unwrap();
        assert_eq!(moved.epic_meta().as_deref(), Some("payments"));
        assert_eq!(moved.epic.as_deref(), Some("payments"));

        // And back out to the loose backlog.
        board.set_epic(&moved, None).unwrap();
        let loose = board.find("001").unwrap();
        assert_eq!(loose.epic, None);
        assert_eq!(loose.epic_meta(), None);
    }

    // -- Leases block moves too (docs/adr/0007-concurrency-and-locking.md) -

    fn leased_backlog_ticket(board: &Board, name: &str, holder: &str, until: &str) {
        let body = format!(
            "---\nid: \"{}\"\ntitle: \"T\"\nlocked-by: \"{holder}\"\nlocked-until: \"{until}\"\n---\n\nbody\n",
            &name[..3]
        );
        fs::write(board.dir(Column::Backlog).join(name), body).unwrap();
    }

    fn future() -> String {
        (time::OffsetDateTime::now_utc() + time::Duration::hours(1))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    #[test]
    fn move_task_refuses_a_ticket_leased_by_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        leased_backlog_ticket(&board, "001-a.md", "other@example.com", &future());
        let task = board.find("001").unwrap();
        assert!(board.move_task(&task, Column::Todo, false).is_err());
        assert_eq!(board.find("001").unwrap().column, Column::Backlog);
    }

    #[test]
    fn move_task_succeeds_when_the_lease_is_your_own() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        leased_backlog_ticket(&board, "001-a.md", "t@t.co", &future());
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Todo, false).unwrap();
        assert_eq!(board.find("001").unwrap().column, Column::Todo);
    }

    #[test]
    fn archive_refuses_a_ticket_leased_by_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        leased_backlog_ticket(&board, "001-a.md", "other@example.com", &future());
        let task = board.find("001").unwrap();
        assert!(board.archive(&task).is_err());
    }

    #[test]
    fn set_epic_refuses_a_ticket_leased_by_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        leased_backlog_ticket(&board, "001-a.md", "other@example.com", &future());
        let task = board.find("001").unwrap();
        assert!(board.set_epic(&task, Some("checkout")).is_err());
    }

    // -- WIP limits ----------------------------------------------------------

    fn set_wip_limit(board: &Board, key: &str, limit: u32) {
        fs::write(
            crate::board_settings::path(board),
            format!("{key} = {limit}\n"),
        )
        .unwrap();
    }

    #[test]
    fn move_task_refuses_when_the_destination_is_already_at_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        backlog_ticket(&board, "001-a.md");
        backlog_ticket(&board, "002-b.md");
        touch(&board, "existing.md"); // one ticket already in todo/
        set_wip_limit(&board, "wip-todo", 1);
        let task = board.find("001").unwrap();
        let err = board
            .move_task(&task, Column::Todo, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("todo") && err.contains('1'), "{err}");
        assert_eq!(board.find("001").unwrap().column, Column::Backlog);
    }

    #[test]
    fn move_task_force_overrides_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        backlog_ticket(&board, "001-a.md");
        touch(&board, "existing.md");
        set_wip_limit(&board, "wip-todo", 1);
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Todo, true).unwrap();
        assert_eq!(board.find("001").unwrap().column, Column::Todo);
    }

    #[test]
    fn moving_out_of_an_over_limit_column_is_never_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        // Two tickets already in todo/, then the limit is lowered to one —
        // the column is now over its limit by hand, the exact case the
        // ticket's own notes call out. Moving one back OUT must still work.
        touch(&board, "001-a.md");
        touch(&board, "002-b.md");
        set_wip_limit(&board, "wip-todo", 1);
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Doing, false).unwrap();
        assert_eq!(board.find("001").unwrap().column, Column::Doing);
    }

    #[test]
    fn a_limit_on_todo_does_not_apply_to_doing() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        backlog_ticket(&board, "001-a.md");
        touch(&board, "existing.md");
        set_wip_limit(&board, "wip-todo", 1);
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Doing, false).unwrap();
        assert_eq!(board.find("001").unwrap().column, Column::Doing);
    }

    // -- Emptying a group ------------------------------------------------

    #[test]
    fn empty_group_moves_every_ticket_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        backlog_ticket(&board, "001-a.md");
        backlog_ticket(&board, "002-b.md");
        board
            .set_epic(&board.find("001").unwrap(), Some("checkout"))
            .unwrap();
        board
            .set_epic(&board.find("002").unwrap(), Some("checkout"))
            .unwrap();
        assert_eq!(board.tasks_in_group("checkout").unwrap().len(), 2);

        let result = board.empty_group("checkout").unwrap();
        assert_eq!(result.moved.len(), 2);
        assert!(result.skipped.is_empty());
        assert_eq!(board.tasks_in_group("checkout").unwrap().len(), 0);
        assert_eq!(board.find("001").unwrap().epic, None);
        assert_eq!(board.find("002").unwrap().epic, None);
    }

    #[test]
    fn empty_group_skips_a_ticket_it_cannot_move_rather_than_aborting() {
        let dir = tempfile::tempdir().unwrap();
        let board = git_repo_board(dir.path());
        let epic_dir = board.dir(Column::Backlog).join("checkout");
        fs::create_dir_all(&epic_dir).unwrap();
        backlog_ticket(&board, "001-a.md");
        board
            .set_epic(&board.find("001").unwrap(), Some("checkout"))
            .unwrap();
        let body = format!(
            "---\nid: \"002\"\ntitle: \"T\"\nlocked-by: \"other@example.com\"\nlocked-until: \"{}\"\n---\n\nbody\n",
            future()
        );
        fs::write(epic_dir.join("002-b.md"), body).unwrap();

        let result = board.empty_group("checkout").unwrap();
        assert_eq!(result.moved.len(), 1, "{:?}", result.moved);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].0, "002-b");
    }

    #[test]
    fn deleting_a_nonempty_group_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        backlog_ticket(&board, "001-a.md");
        board
            .set_epic(&board.find("001").unwrap(), Some("checkout"))
            .unwrap();

        let err = board.delete_group("checkout").unwrap_err();
        assert!(err.to_string().contains("1 ticket"), "{err}");
        assert!(
            board.group("checkout").is_ok(),
            "must not have been deleted"
        );
    }

    #[test]
    fn deleting_a_nonempty_group_counts_tickets_outside_the_backlog_too() {
        // A committed sprint's tickets can sit in doing/done while the
        // folder itself is physically empty — delete has to see them the
        // same way empty_group already does, or it would silently orphan
        // their epic reference.
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        sprint(&board, "sprint-1");
        backlog_ticket(&board, "001-a.md");
        let task = board.find("001").unwrap();
        task.set_points(task.hash().unwrap(), "3").unwrap();
        board.set_epic(&task, Some("sprint-1")).unwrap();
        let task = board.find("001").unwrap();
        board.move_task(&task, Column::Doing, false).unwrap();

        let err = board.delete_group("sprint-1").unwrap_err();
        assert!(err.to_string().contains("1 ticket"), "{err}");
    }

    #[test]
    fn an_emptied_group_can_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::create_dir_all(board.dir(Column::Backlog).join("checkout")).unwrap();
        backlog_ticket(&board, "001-a.md");
        board
            .set_epic(&board.find("001").unwrap(), Some("checkout"))
            .unwrap();

        board.empty_group("checkout").unwrap();
        board.delete_group("checkout").unwrap();
        assert!(board.group("checkout").is_err(), "the directory is gone");
    }

    #[test]
    fn deleting_an_unknown_group_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        assert!(board.delete_group("nope").is_err());
    }

    #[test]
    fn tasks_tagged_filters_across_every_column() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        fs::write(
            board.dir(Column::Backlog).join("001-a.md"),
            "---\nid: \"001\"\ntitle: \"A\"\ntags: [DUPLICATE]\n---\n\nbody\n",
        )
        .unwrap();
        fs::write(
            board.dir(Column::Done).join("002-b.md"),
            "---\nid: \"002\"\ntitle: \"B\"\ntags: [DUPLICATE,urgent]\n---\n\nbody\n",
        )
        .unwrap();
        fs::write(
            board.dir(Column::Todo).join("003-c.md"),
            "---\nid: \"003\"\ntitle: \"C\"\ntags: []\n---\n\nbody\n",
        )
        .unwrap();

        let tagged = board.tasks_tagged("DUPLICATE").unwrap();
        assert_eq!(
            tagged.iter().map(|t| t.id).collect::<Vec<_>>(),
            [Some(1), Some(2)]
        );
        assert_eq!(board.tasks_tagged("urgent").unwrap().len(), 1);
        assert!(board.tasks_tagged("missing").unwrap().is_empty());
    }

    #[test]
    fn a_new_board_starts_at_one() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        assert_eq!(board.next_id().unwrap(), 1);
        assert_eq!(
            crate::board_settings::load(&board).next_id(),
            Some(1),
            "the counter lives in board-local settings, not the legacy file"
        );
        assert!(
            !board.legacy_counter_path().exists(),
            "a brand new board never writes the legacy .next-id file"
        );
    }

    #[test]
    fn a_legacy_next_id_file_is_migrated_into_settings_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        touch(&board, "007-seven.md");
        // A board that predates the move to board-local settings: no
        // `next-id` key there, only the old file.
        fs::remove_file(crate::board_settings::path(&board)).unwrap();
        fs::write(board.legacy_counter_path(), "9\n").unwrap();

        assert_eq!(board.next_id().unwrap(), 9);
        assert_eq!(
            crate::board_settings::load(&board).next_id(),
            Some(9),
            "the value should now live in board-local settings"
        );
        assert!(
            !board.legacy_counter_path().exists(),
            "the legacy file should be gone once migrated, not kept around forever"
        );

        // Reading again (the ordinary case: a second command against the
        // same, now-migrated board) must not require the legacy file.
        touch(&board, "009-nine.md");
        board.record_id(9).unwrap();
        assert_eq!(board.next_id().unwrap(), 10);
    }

    #[test]
    fn the_counter_moves_on_as_tickets_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        touch(&board, "001-one.md");
        board.record_id(1).unwrap();
        assert_eq!(board.next_id().unwrap(), 2);
        touch(&board, "002-two.md");
        board.record_id(2).unwrap();
        assert_eq!(board.next_id().unwrap(), 3);
    }

    #[test]
    fn a_deleted_ticket_does_not_hand_its_id_back() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        touch(&board, "001-one.md");
        board.record_id(1).unwrap();
        // Deleted outright, not archived: nothing on disk remembers it.
        fs::remove_file(board.dir(Column::Todo).join("001-one.md")).unwrap();
        assert_eq!(board.next_id().unwrap(), 2, "the counter still knows");
    }

    #[test]
    fn a_lost_or_stale_counter_cannot_collide() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        touch(&board, "007-seven.md");
        // Counter behind what's on disk — a bad merge, or a hand-added ticket.
        let mut settings = crate::board_settings::load(&board);
        settings.set_next_id(3);
        settings.save(&board).unwrap();
        assert_eq!(board.next_id().unwrap(), 8);
        // Counter missing entirely — the settings file itself is gone.
        fs::remove_file(crate::board_settings::path(&board)).unwrap();
        assert_eq!(board.next_id().unwrap(), 8);
    }

    #[test]
    fn recording_an_older_id_does_not_wind_the_counter_back() {
        let dir = tempfile::tempdir().unwrap();
        let board = board_at(dir.path());
        board.record_id(9).unwrap();
        board.record_id(2).unwrap();
        assert_eq!(board.next_id().unwrap(), 10);
    }

    #[test]
    fn columns_parse_case_insensitively() {
        assert_eq!(Column::parse("DONE"), Some(Column::Done));
        assert_eq!(Column::parse("todo"), Some(Column::Todo));
        assert_eq!(Column::parse("Backlog"), Some(Column::Backlog));
        assert_eq!(Column::parse("archive"), None);
    }

    /// A child that qualifies as a project sibling: a git repo with a board
    /// of its own. `project_siblings` only checks for directories, not a
    /// real git repository, so a bare `.git` dir is enough here.
    fn project_child(parent: &Path, name: &str) {
        let child = parent.join(name);
        fs::create_dir_all(child.join(".git")).unwrap();
        board_at(&child);
    }

    #[test]
    fn project_siblings_finds_nothing_in_an_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_siblings(dir.path()).is_empty());
    }

    #[test]
    fn project_siblings_ignores_plain_directories_and_half_qualified_ones() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("not-a-repo")).unwrap();
        // A git repo with no board of its own.
        fs::create_dir_all(dir.path().join("code-only/.git")).unwrap();
        // A board-shaped directory that isn't a git repository.
        board_at(&dir.path().join("board-but-no-git"));
        assert!(project_siblings(dir.path()).is_empty());
    }

    #[test]
    fn project_siblings_finds_the_one_qualifying_child() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("not-a-repo")).unwrap();
        project_child(dir.path(), "acme-tickets");
        let found = project_siblings(dir.path());
        assert_eq!(found, vec![dir.path().join("acme-tickets")]);
    }

    #[test]
    fn project_siblings_finds_every_qualifying_child_sorted() {
        let dir = tempfile::tempdir().unwrap();
        project_child(dir.path(), "widgets-tickets");
        project_child(dir.path(), "acme-tickets");
        let found = project_siblings(dir.path());
        assert_eq!(
            found,
            vec![
                dir.path().join("acme-tickets"),
                dir.path().join("widgets-tickets"),
            ]
        );
    }
}
