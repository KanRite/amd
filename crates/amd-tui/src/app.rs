//! Board state and key handling. Nothing here draws or touches the terminal.

use std::cell::Cell;
use std::collections::BTreeSet;

use agile_md::board::Column;
use agile_md::model::{self, RepoBoard};
use agile_md::transition::{self, Event};

use crate::form::{Form, Outcome, Target};
use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where one ticket sits: which repository, and its index in that
/// repository's lane for the column being asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub repo: usize,
    pub card: usize,
}

/// The sections of the filter screen, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Repos,
    Tags,
    Groups,
}

impl Section {
    const ALL: [Section; 3] = [Section::Repos, Section::Tags, Section::Groups];

    fn step(self, forward: bool) -> Section {
        let at = Section::ALL.iter().position(|s| *s == self).unwrap_or(0);
        let len = Section::ALL.len();
        let next = if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        };
        Section::ALL[next]
    }
}

/// Which screen is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Board,
    /// A ticket, read-only.
    Ticket {
        lines: Vec<String>,
        scroll: u16,
    },
    Filter {
        section: Section,
        cursor: usize,
    },
    /// Making or editing a ticket.
    Form(Box<Form>),
    Help,
}

pub struct App {
    pub repos: Vec<RepoBoard>,
    /// Declared repositories with no usable local clone — named in the header
    /// so a teammate's repository is visible before it is cloned.
    pub unregistered: Vec<String>,
    pub mode: Mode,
    /// Selected column, an index into `Column::ALL`.
    pub col: usize,
    /// Selected row within `visible(col)`.
    pub row: usize,
    /// Tickets must carry one of these tags; empty means no tag filter.
    pub tags: BTreeSet<String>,
    /// Only tickets in this epic or sprint; `None` means no group filter.
    pub group: Option<String>,
    pub status: Option<String>,
    pub quit: bool,
    /// Scroll offset each column's list settled on last frame. The renderer
    /// owns it (ratatui's `ListState` decides it during render) but key
    /// handling and any later click handling need it back, and the renderer
    /// only gets `&App`, hence `Cell`.
    pub offsets: [Cell<usize>; 4],
}

impl App {
    /// Open the current project's board and every declared repository.
    pub fn load() -> Result<Self> {
        let model::Repos {
            repos,
            unregistered,
            ..
        } = model::load_repos()?;
        Ok(Self::new(repos, unregistered))
    }

    pub fn new(repos: Vec<RepoBoard>, unregistered: Vec<String>) -> Self {
        App {
            repos,
            unregistered,
            mode: Mode::Board,
            col: 1,
            row: 0,
            tags: BTreeSet::new(),
            group: None,
            status: None,
            quit: false,
            offsets: Default::default(),
        }
    }

    /// The tickets showing in a column: every visible repository's, filtered
    /// by tag and group, repositories in order and each repository's own
    /// ranking kept.
    pub fn visible(&self, column: usize) -> Vec<Entry> {
        let mut entries = Vec::new();
        for (repo, board) in self.repos.iter().enumerate() {
            if !board.visible {
                continue;
            }
            for (card, ticket) in board.lanes[column].iter().enumerate() {
                if !model::card_visible(ticket, &self.tags) {
                    continue;
                }
                if let Some(group) = &self.group
                    && !model::card_in_group(ticket, group)
                {
                    continue;
                }
                entries.push(Entry { repo, card });
            }
        }
        entries
    }

    /// The selected ticket, if the selected column has one.
    pub fn selected(&self) -> Option<Entry> {
        self.visible(self.col).get(self.row).copied()
    }

    /// Whether more than one repository is showing — only then does a ticket
    /// need its repository's name beside it, since ids restart in each.
    pub fn several_repos(&self) -> bool {
        self.repos.iter().filter(|repo| repo.visible).count() > 1
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        // A message lives until the next key; whatever that key does may set a
        // fresh one.
        self.status = None;
        if let Mode::Form(form) = &mut self.mode {
            match form.handle_key(key) {
                Outcome::Editing => {}
                Outcome::Cancel => self.mode = Mode::Board,
                Outcome::Save => self.save_form(),
            }
            return;
        }
        match self.mode.clone() {
            Mode::Board => self.board_key(key.code),
            Mode::Ticket { .. } => self.ticket_key(key.code),
            Mode::Filter { section, cursor } => self.filter_key(key.code, section, cursor),
            Mode::Help => self.mode = Mode::Board,
            Mode::Form(_) => {}
        }
    }

    fn board_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('h') | KeyCode::Left => self.select_column(self.col.saturating_sub(1)),
            KeyCode::Char('l') | KeyCode::Right => {
                self.select_column((self.col + 1).min(Column::ALL.len() - 1));
            }
            KeyCode::Char('j') | KeyCode::Down => self.select_row(self.row + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select_row(self.row.saturating_sub(1)),
            KeyCode::Char('g') | KeyCode::Home => self.select_row(0),
            KeyCode::Char('G') | KeyCode::End => self.select_row(usize::MAX),
            KeyCode::Char('H') | KeyCode::Char('<') => self.move_selected(-1),
            KeyCode::Char('L') | KeyCode::Char('>') => self.move_selected(1),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('n') => self.start_new(),
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('r') => {
                self.reload();
                self.status = Some("reloaded".to_string());
            }
            KeyCode::Char('f') => {
                self.mode = Mode::Filter {
                    section: Section::Repos,
                    cursor: 0,
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }

    fn select_column(&mut self, col: usize) {
        self.col = col;
        self.select_row(self.row);
    }

    /// Select a row, clamped to what the column holds.
    fn select_row(&mut self, row: usize) {
        let count = self.visible(self.col).len();
        self.row = row.min(count.saturating_sub(1));
    }

    /// Move the selected ticket one column left or right through
    /// `transition`, the same path as `amd todo`/`doing`/`done`. A refusal (a
    /// WIP limit, someone else's lease) lands on the status line and moves
    /// nothing.
    pub fn move_selected(&mut self, delta: isize) {
        let Some(entry) = self.selected() else {
            return;
        };
        let target = self.col as isize + delta;
        if target < 0 || target >= Column::ALL.len() as isize {
            self.status = Some(format!(
                "already in the {} column",
                if delta < 0 { "first" } else { "last" }
            ));
            return;
        }
        let target = target as usize;
        let to = Column::ALL[target];
        let repo = &self.repos[entry.repo];
        let ticket = &repo.lanes[self.col][entry.card];
        let stem = ticket.task.stem.clone();
        let result = if to == Column::Todo {
            transition::enter_todo(&repo.board, &ticket.task, &Default::default())
        } else {
            transition::move_to(&repo.board, &ticket.task, to, false)
        };
        match result {
            Ok(outcome) => {
                self.status = Some(summarise(&outcome.events));
                self.reload();
                self.col = target;
                // Follow the ticket into its new column.
                let row = self.visible(target).iter().position(|e| {
                    e.repo == entry.repo
                        && self.repos[e.repo].lanes[target][e.card].task.stem == stem
                });
                self.select_row(row.unwrap_or(self.row));
            }
            Err(err) => self.status = Some(format!("{err:#}")),
        }
    }

    /// The names of a repository's epics and sprints, for the form's choice.
    fn group_names(&self, repo: usize) -> Vec<String> {
        self.repos[repo]
            .groups
            .iter()
            .filter_map(|(group, _)| group.as_ref().map(|group| group.name.clone()))
            .collect()
    }

    /// Open a blank form. The ticket goes to the selected ticket's repository
    /// (or the first one showing), and into the group being filtered on.
    fn start_new(&mut self) {
        let repo = self
            .selected()
            .map(|entry| entry.repo)
            .or_else(|| self.repos.iter().position(|repo| repo.visible))
            .unwrap_or(0);
        let groups = self.group_names(repo);
        self.mode = Mode::Form(Box::new(Form::new_ticket(
            repo,
            &groups,
            self.group.as_deref(),
        )));
    }

    /// Open the selected ticket in the form, fingerprinted so a save notices
    /// if the file changed in the meantime.
    fn start_edit(&mut self) {
        let Some(entry) = self.selected() else {
            return;
        };
        let task = &self.repos[entry.repo].lanes[self.col][entry.card].task;
        let groups = self.group_names(entry.repo);
        match (task.hash(), task.body()) {
            (Ok(hash), Ok(body)) => {
                self.mode = Mode::Form(Box::new(Form::edit_ticket(
                    entry.repo, task, &groups, hash, body,
                )));
            }
            (Err(err), _) | (_, Err(err)) => self.status = Some(format!("{err:#}")),
        }
    }

    /// Save the open form through the library — the same path `amd new` and a
    /// field edit take. A refusal (an unknown parent, a stale file) keeps the
    /// form open with the reason, so nothing typed is lost.
    fn save_form(&mut self) {
        let Mode::Form(form) = std::mem::replace(&mut self.mode, Mode::Board) else {
            return;
        };
        let fields = form.to_fields();
        let (repo, result) = match &form.target {
            Target::New { repo } => (
                *repo,
                transition::create_ticket(&self.repos[*repo].board, &fields),
            ),
            Target::Edit { repo, stem, hash } => {
                let board = &self.repos[*repo].board;
                (
                    *repo,
                    board
                        .find(stem)
                        .and_then(|task| transition::edit_ticket(board, &task, *hash, &fields)),
                )
            }
        };
        match result {
            Ok(outcome) => {
                self.status = Some(summarise(&outcome.events));
                let stem = match &form.target {
                    Target::Edit { stem, .. } => Some(stem.clone()),
                    Target::New { .. } => outcome
                        .dest
                        .as_ref()
                        .and_then(|path| path.file_stem())
                        .map(|stem| stem.to_string_lossy().into_owned()),
                };
                let status = self.status.take();
                self.reload();
                self.status = status;
                if let Some(stem) = stem {
                    self.select_ticket(repo, &stem);
                }
            }
            Err(err) => {
                let mut form = form;
                form.error = Some(format!("{err:#}"));
                self.mode = Mode::Form(form);
            }
        }
    }

    /// Put the selection on a ticket, wherever it now is, if the filters let
    /// it show.
    fn select_ticket(&mut self, repo: usize, stem: &str) {
        for col in 0..Column::ALL.len() {
            let found = self.visible(col).iter().position(|entry| {
                entry.repo == repo && self.repos[repo].lanes[col][entry.card].task.stem == stem
            });
            if let Some(row) = found {
                self.col = col;
                self.row = row;
                return;
            }
        }
    }

    fn open_selected(&mut self) {
        let Some(entry) = self.selected() else {
            return;
        };
        let ticket = &self.repos[entry.repo].lanes[self.col][entry.card];
        let task = &ticket.task;
        let mut lines = vec![
            format!("{}  {}", ticket.id, ticket.title),
            String::new(),
            format!("column    {}", task.column),
            format!("assignee  {}", ticket.assignee.as_deref().unwrap_or("-")),
            format!("points    {}", ticket.points.as_deref().unwrap_or("-")),
            format!("tags      {}", join_or_dash(&task.tags())),
        ];
        if let Some(branch) = task.branch() {
            lines.push(format!("branch    {branch}"));
        }
        if let Some(epic) = task.epic_meta().or_else(|| task.epic.clone()) {
            lines.push(format!("group     {epic}"));
        }
        lines.push(String::new());
        match task.body() {
            Ok(body) => lines.extend(body.lines().map(str::to_string)),
            Err(err) => lines.push(format!("{err:#}")),
        }
        self.mode = Mode::Ticket { lines, scroll: 0 };
    }

    fn ticket_key(&mut self, code: KeyCode) {
        let Mode::Ticket { lines, scroll } = &mut self.mode else {
            return;
        };
        let last = lines.len().saturating_sub(1) as u16;
        match code {
            KeyCode::Char('j') | KeyCode::Down => *scroll = (*scroll + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
            KeyCode::Char('G') | KeyCode::End => *scroll = last,
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => self.mode = Mode::Board,
            _ => {}
        }
    }

    /// Every tag across the visible repositories' tickets, sorted.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags = BTreeSet::new();
        for repo in &self.repos {
            for lane in &repo.lanes {
                for card in lane {
                    tags.extend(card.task.tags());
                }
            }
        }
        tags.into_iter().collect()
    }

    /// Every epic and sprint name across the repositories, sorted.
    pub fn all_groups(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for repo in &self.repos {
            for (group, _) in &repo.groups {
                if let Some(group) = group {
                    names.insert(group.name.clone());
                }
            }
        }
        names.into_iter().collect()
    }

    /// How many rows a filter section lists. Groups lead with a "no group
    /// filter" row.
    fn section_len(&self, section: Section) -> usize {
        match section {
            Section::Repos => self.repos.len(),
            Section::Tags => self.all_tags().len(),
            Section::Groups => self.all_groups().len() + 1,
        }
    }

    fn filter_key(&mut self, code: KeyCode, section: Section, cursor: usize) {
        let len = self.section_len(section);
        let set = |app: &mut App, section, cursor| app.mode = Mode::Filter { section, cursor };
        match code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('f') => {
                self.mode = Mode::Board;
                // The filter may have emptied the column the cursor was in.
                self.select_row(self.row);
            }
            KeyCode::Tab => set(self, section.step(true), 0),
            KeyCode::BackTab => set(self, section.step(false), 0),
            KeyCode::Char('j') | KeyCode::Down => {
                set(self, section, (cursor + 1).min(len.saturating_sub(1)));
            }
            KeyCode::Char('k') | KeyCode::Up => set(self, section, cursor.saturating_sub(1)),
            KeyCode::Char('c') => {
                self.tags.clear();
                self.group = None;
                for repo in &mut self.repos {
                    repo.visible = true;
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle(section, cursor),
            _ => {}
        }
    }

    fn toggle(&mut self, section: Section, cursor: usize) {
        match section {
            Section::Repos => {
                if let Some(repo) = self.repos.get_mut(cursor) {
                    repo.visible = !repo.visible;
                }
            }
            Section::Tags => {
                if let Some(tag) = self.all_tags().get(cursor)
                    && !self.tags.remove(tag)
                {
                    self.tags.insert(tag.clone());
                }
            }
            // Row 0 is "no filter"; the rest are group names. Picking the
            // current group again clears it, like a tag.
            Section::Groups => {
                self.group = match cursor.checked_sub(1) {
                    None => None,
                    Some(at) => self
                        .all_groups()
                        .get(at)
                        .filter(|name| self.group.as_ref() != Some(name))
                        .cloned(),
                };
            }
        }
    }

    /// Re-read every open board. Doesn't re-derive `unregistered`: a repository
    /// registered while the board is open shows up on the next launch, as in
    /// the desktop board.
    pub fn reload(&mut self) {
        for repo in &mut self.repos {
            match (
                model::read_lanes(&repo.board),
                model::read_groups(&repo.board),
            ) {
                (Ok(lanes), Ok(groups)) => {
                    repo.lanes = lanes;
                    repo.groups = groups;
                }
                (Err(err), _) | (_, Err(err)) => self.status = Some(format!("{err:#}")),
            }
        }
        // A reload can drop the filter's group or shrink the column.
        if let Some(group) = &self.group
            && !self.all_groups().contains(group)
        {
            self.group = None;
        }
        self.select_row(self.row);
    }
}

/// Everything a transition reported, as one status line.
fn summarise(events: &[Event]) -> String {
    events
        .iter()
        .map(|event| match event {
            Event::Info(text) => text.clone(),
            Event::Warn(text) => format!("warning: {text}"),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;
    use crate::fixture::repo;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            app.handle_key(key(c));
        }
    }

    fn app_in(dir: &std::path::Path) -> App {
        let repo = repo(
            dir,
            &[
                ("001-a.md", Column::Todo, "ux"),
                ("002-b.md", Column::Todo, "cli"),
                ("003-c.md", Column::Doing, "ux"),
            ],
        );
        App::new(vec![repo], Vec::new())
    }

    #[test]
    fn it_opens_on_todo_with_the_first_ticket_selected() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in(dir.path());
        assert_eq!(app.col, 1);
        assert_eq!(app.visible(1).len(), 2);
        assert_eq!(app.selected(), Some(Entry { repo: 0, card: 0 }));
    }

    #[test]
    fn selection_clamps_to_what_a_column_holds() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "jjjj");
        assert_eq!(app.row, 1, "two tickets in todo");
        press(&mut app, "l");
        assert_eq!(app.col, 2);
        assert_eq!(app.row, 0, "one ticket in doing");
        press(&mut app, "ll");
        assert_eq!(app.col, 3, "stops at the last column");
        assert_eq!(app.selected(), None, "done is empty");
    }

    #[test]
    fn moving_a_ticket_right_moves_the_file_and_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        // Doing already holds a ticket and the default limit is 1, so clear
        // the way: move 003 on to done first.
        press(&mut app, "lL");
        assert!(dir.path().join("tasks/done/003-c.md").exists());
        assert_eq!(app.col, 3, "the selection followed 003 into done");
        press(&mut app, "hhL");
        assert!(dir.path().join("tasks/doing/001-a.md").exists());
        assert_eq!(app.col, 2, "the selection followed the ticket");
        assert_eq!(app.selected().map(|e| e.card), Some(0));
        assert!(
            app.status.as_deref().is_some_and(|s| s.contains("moved")),
            "{:?}",
            app.status
        );
    }

    #[test]
    fn a_refused_move_changes_nothing_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        // 003 holds doing at its default limit of one.
        press(&mut app, "L");
        assert!(dir.path().join("tasks/todo/001-a.md").exists());
        assert_eq!(app.col, 1);
        let status = app.status.clone().unwrap_or_default();
        assert!(status.contains("1 of 1 allowed"), "{status}");
    }

    #[test]
    fn moving_off_either_end_is_a_note_not_a_move() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(
            dir.path(),
            &[
                ("001-a.md", Column::Backlog, ""),
                ("002-b.md", Column::Done, ""),
            ],
        );
        let mut app = App::new(vec![repo], Vec::new());
        press(&mut app, "hH");
        assert_eq!(
            (app.col, app.status.as_deref()),
            (0, Some("already in the first column"))
        );
        press(&mut app, "lllL");
        assert_eq!(
            (app.col, app.status.as_deref()),
            (3, Some("already in the last column"))
        );
        assert!(dir.path().join("tasks/backlog/001-a.md").exists());
        assert!(dir.path().join("tasks/done/002-b.md").exists());
    }

    #[test]
    fn a_tag_filter_hides_the_other_tickets() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        // f, Tab to tags, Space on the first tag ("cli"), then back out.
        press(&mut app, "f");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        press(&mut app, " ");
        press(&mut app, "f");
        assert_eq!(app.tags.iter().next().map(String::as_str), Some("cli"));
        assert_eq!(app.visible(1).len(), 1);
        assert_eq!(app.visible(2).len(), 0, "003 is tagged ux");
    }

    #[test]
    fn hiding_a_repository_removes_its_tickets() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "f ");
        assert!(!app.repos[0].visible);
        assert!(app.visible(1).is_empty());
        press(&mut app, "c");
        assert!(app.repos[0].visible, "c clears every filter");
    }

    #[test]
    fn enter_opens_the_ticket_and_escape_closes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match &app.mode {
            Mode::Ticket { lines, .. } => {
                assert!(lines[0].contains("T001"), "{lines:?}");
                assert!(lines.iter().any(|l| l.contains("body of 001")));
            }
            other => panic!("expected the ticket view, got {other:?}"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Board);
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "q");
        assert!(app.quit);
        let mut app = app_in(dir.path());
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.quit);
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn special(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn a_new_ticket_is_written_to_the_backlog_and_selected() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "n");
        assert!(matches!(app.mode, Mode::Form(_)));
        press(&mut app, "Brand new");
        ctrl(&mut app, 's');
        assert_eq!(app.mode, Mode::Board, "{:?}", app.status);
        assert!(dir.path().join("tasks/backlog/004-brand-new.md").exists());
        assert_eq!(app.col, 0, "the selection went to the new ticket");
        let entry = app.selected().unwrap();
        assert_eq!(app.repos[0].lanes[0][entry.card].title, "Brand new");
        assert!(
            app.status.as_deref().is_some_and(|s| s.contains("created")),
            "{:?}",
            app.status
        );
    }

    #[test]
    fn saving_a_new_ticket_with_no_title_keeps_the_form_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "n");
        ctrl(&mut app, 's');
        match &app.mode {
            Mode::Form(form) => assert!(form.error.is_some(), "no reason given"),
            other => panic!("the form should still be open, got {other:?}"),
        }
        assert!(app.repos[0].lanes[0].is_empty(), "nothing was created");
        app.reload();
        assert!(app.repos[0].lanes[0].is_empty(), "nothing on disk either");
    }

    #[test]
    fn editing_opens_filled_in_and_saves_over_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "e");
        let Mode::Form(form) = &app.mode else {
            panic!("expected the form");
        };
        assert_eq!(form.to_fields().title, "T001");
        press(&mut app, " edited");
        ctrl(&mut app, 's');
        assert_eq!(app.mode, Mode::Board, "{:?}", app.status);
        let entry = app.selected().unwrap();
        assert_eq!(app.repos[0].lanes[1][entry.card].title, "T001 edited");
        assert!(dir.path().join("tasks/todo/001-a.md").exists());
    }

    #[test]
    fn a_ticket_changed_while_the_form_was_open_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "e");
        // Someone else saves the ticket first.
        let path = dir.path().join("tasks/todo/001-a.md");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("T001", "Theirs")).unwrap();
        press(&mut app, " mine");
        ctrl(&mut app, 's');
        match &app.mode {
            Mode::Form(form) => assert!(form.error.is_some()),
            other => panic!("the form should still be open, got {other:?}"),
        }
        assert!(std::fs::read_to_string(&path).unwrap().contains("Theirs"));
    }

    #[test]
    fn escape_discards_the_form() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "nabandoned");
        special(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Board);
        assert!(app.repos[0].lanes[0].is_empty(), "nothing was created");
        app.reload();
        assert!(app.repos[0].lanes[0].is_empty(), "nothing on disk either");
    }

    #[test]
    fn editing_with_nothing_selected_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(dir.path());
        press(&mut app, "lle");
        assert_eq!(app.mode, Mode::Board, "done is empty");
    }
}
