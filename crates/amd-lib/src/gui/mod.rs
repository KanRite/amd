//! The desktop board.
//!
//! A conventional Kanban: each status is a column, side by side, with its
//! tickets stacked top to bottom. Dragging a card **up or down reorders** it
//! within its status; dragging it **across changes** its status. One window
//! shows every registered repository, so work across boards is visible
//! together without a web front end.

pub mod richtext;
pub mod settings;

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::board::{Board, Column};
use crate::group::{DEFAULT_DAYS, Group, Kind, State};
use crate::registry::Registry;
use crate::task::{ContentHash, Task};
use settings::{Accent, FontSize, GuiSettings, Theme};

/// Wide enough for a title of a few words without wrapping to three lines,
/// narrow enough that three columns and a scrollbar fit a laptop screen.
const COLUMN_WIDTH: f32 = 260.0;

/// A card being dragged. Carries where it came from, so the drop can tell a
/// reorder from a status change — and refuse a move between repositories,
/// which would need a new id rather than a rename.
#[derive(Clone)]
struct Dragged {
    path: PathBuf,
    repo: usize,
}

/// What a completed drag asks the board to do, applied after the frame so the
/// UI is never mutating files mid-draw.
enum Drop {
    /// Land `path` in `column` of `repo`, at position `index` among the cards
    /// already there.
    Land {
        path: PathBuf,
        repo: usize,
        column: Column,
        index: usize,
    },
}

struct Card {
    task: Task,
    title: String,
    id: String,
    assignee: Option<String>,
    points: Option<String>,
    /// Sort key: the hand-set order when there is one, otherwise the id, so a
    /// board nobody has dragged still reads in id order.
    rank: f64,
}

struct RepoBoard {
    name: String,
    board: Board,
    lanes: Vec<Vec<Card>>,
    /// Backlog tickets grouped by folder. `None` is the loose backlog —
    /// everything not filed under an epic or sprint yet.
    groups: Vec<(Option<Group>, Vec<Card>)>,
    visible: bool,
}

/// Which of the two boards is on screen. The board is the committed work;
/// the backlog is everything else, grouped by epic.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Board,
    Backlog,
}

/// A ticket being written in the new-ticket overlay.
struct Draft {
    repo: usize,
    title: String,
    epic: Option<String>,
    points: String,
    /// Notes written while creating, appended to the template's body.
    body: String,
    markdown: bool,
    line: richtext::Editing,
}

/// An epic or sprint being created or edited. The same overlay does both:
/// creating differs only in that the name is still editable.
struct GroupDraft {
    repo: usize,
    kind: Kind,
    /// `None` while creating; the existing directory name when editing.
    editing: Option<String>,
    name: String,
    description: String,
    days: String,
    state: State,
}

/// The ticket open in the editor overlay. Holds its own copy of the fields, so
/// closing without saving costs nothing and the board underneath keeps showing
/// what is actually on disk.
struct Editor {
    repo: usize,
    path: PathBuf,
    id: String,
    title: String,
    assignee: String,
    points: String,
    body: String,
    /// Fingerprint of the ticket's content when the editor opened. Saving
    /// checks this before writing — see `Task::apply_edit` and
    /// `docs/adr/0007-concurrency-and-locking.md`. A conflict means the
    /// ticket changed elsewhere while this was open; the fields above are
    /// stale and saving them would silently discard whatever changed it.
    hash: ContentHash,
    /// Rich text unless asked otherwise — the markdown is there for when you
    /// want it, not as the price of writing a sentence.
    markdown: bool,
    /// Which line the rich-text editor has open, kept between frames.
    line: richtext::Editing,
}

impl Editor {
    fn open(repo: usize, card: &Card) -> Result<Self> {
        Ok(Self {
            repo,
            path: card.task.path.clone(),
            id: card.id.clone(),
            title: card.task.title(),
            assignee: card.task.assignee().unwrap_or_default(),
            points: card.task.points().unwrap_or_default(),
            body: card.task.body()?,
            hash: card.task.hash()?,
            markdown: false,
            line: None,
        })
    }
}

pub struct BoardApp {
    repos: Vec<RepoBoard>,
    status: Option<String>,
    editing: Option<Editor>,
    drafting: Option<Draft>,
    group_draft: Option<GroupDraft>,
    view: View,
    settings: GuiSettings,
    /// Open while the settings overlay is up.
    showing_settings: bool,
    /// Which repository the backlog view is showing. The board view stacks
    /// every repository; the backlog is long enough that one at a time reads
    /// better.
    backlog_repo: usize,
    /// Tags currently toggled on in the toolbar. Empty means unfiltered.
    /// A card matches when it carries any tag in the set (OR, not AND) —
    /// narrowing further is a second click away by toggling another tag off.
    tag_filter: BTreeSet<String>,
    /// `git config user.email`, resolved once at load rather than per frame
    /// or per card — `git::config` shells out, and egui redraws
    /// continuously. Used only to tell your own lease apart from someone
    /// else's on a card's badge.
    identity: Option<String>,
}

/// Whether `card` should be drawn under the current tag filter. An empty
/// filter matches everything, same as no filter at all.
fn card_visible(card: &Card, filter: &BTreeSet<String>) -> bool {
    filter.is_empty() || card.task.tags().iter().any(|tag| filter.contains(tag))
}

impl BoardApp {
    pub fn load() -> Result<Self> {
        let current = Board::locate().ok();
        let registry = Registry::load();
        let entries = registry.boards(current.as_ref());

        let mut repos = Vec::new();
        for entry in entries {
            let board = entry.board();
            if !entry.has_board() {
                continue;
            }
            repos.push(RepoBoard {
                // The registry's name, not Board::name(): a board's root is
                // <repo>/tasks, so the latter labels every repository "tasks".
                name: entry.name.clone(),
                lanes: read_lanes(&board)?,
                groups: read_groups(&board)?,
                board,
                visible: true,
            });
        }
        if repos.is_empty() {
            return Err(anyhow!(
                "no boards found — run `amd init` here, or `amd repos add` to register one"
            ));
        }
        // `user.email` is ordinarily global, so any registered repo's
        // context resolves the same identity — the first one is as good as
        // any other.
        let identity = crate::git::config(&repos[0].board.root, "user.email");
        Ok(Self {
            repos,
            status: None,
            editing: None,
            drafting: None,
            group_draft: None,
            view: View::Board,
            settings: GuiSettings::load(),
            showing_settings: false,
            backlog_repo: 0,
            tag_filter: BTreeSet::new(),
            identity,
        })
    }

    fn reload(&mut self) {
        for repo in &mut self.repos {
            match (read_lanes(&repo.board), read_groups(&repo.board)) {
                (Ok(lanes), Ok(groups)) => {
                    repo.lanes = lanes;
                    repo.groups = groups;
                }
                (Err(err), _) | (_, Err(err)) => self.status = Some(format!("{err}")),
            }
        }
    }

    /// File a backlog ticket under an epic, or back into the loose backlog.
    fn refile(&mut self, path: &std::path::Path, repo: usize, epic: Option<String>) {
        let outcome = (|| -> Result<()> {
            let board = &self.repos[repo].board;
            let task = Task::from_path(path, Column::Backlog)
                .ok_or_else(|| anyhow!("{} is not a task", path.display()))?;
            let found = board.find(&task.stem)?;
            board.set_epic(&found, epic.as_deref())?;
            Ok(())
        })();
        match outcome {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("{err}")),
        }
        self.reload();
    }

    /// Write the editor's fields back to the ticket in one call
    /// (`Task::apply_edit`) — not four separate setters, because the hash
    /// captured when the editor opened can only ever match the file once.
    /// A conflict here means the ticket changed elsewhere while this was
    /// open; the status line reports it rather than silently overwriting
    /// whatever changed it.
    fn save(&mut self) {
        let Some(editor) = &self.editing else { return };
        let outcome = (|| -> Result<()> {
            let task = self.repos[editor.repo]
                .lanes
                .iter()
                .flatten()
                .find(|card| card.task.path == editor.path)
                .map(|card| &card.task)
                .ok_or_else(|| anyhow!("{} is no longer on the board", editor.path.display()))?;
            task.apply_edit(
                editor.hash,
                &editor.title,
                &editor.assignee,
                &editor.points,
                &editor.body,
            )
        })();

        match outcome {
            Ok(()) => {
                self.status = None;
                self.editing = None;
                self.reload();
            }
            Err(err) => self.status = Some(format!("{err}")),
        }
    }

    /// Apply a drop: move between columns when the status changed, and always
    /// re-rank so the card keeps the position it was dropped at.
    fn apply(&mut self, drop: Drop) {
        let Drop::Land {
            path,
            repo,
            column,
            index,
        } = drop;

        let Some(target) = self.repos.get(repo) else {
            return;
        };
        // Rank between the neighbours at the drop point, ignoring the card
        // itself so dragging within a lane doesn't measure against its old
        // position.
        let lane: Vec<f64> = target.lanes[column as usize]
            .iter()
            .filter(|card| card.task.path != path)
            .map(|card| card.rank)
            .collect();
        let index = index.min(lane.len());
        let rank = match (
            index.checked_sub(1).and_then(|i| lane.get(i)),
            lane.get(index),
        ) {
            (Some(before), Some(after)) => (before + after) / 2.0,
            (Some(before), None) => before + 1.0,
            (None, Some(after)) => after - 1.0,
            (None, None) => 0.0,
        };

        let mut lease_note = None;
        let outcome = (|| -> Result<()> {
            let task = Task::from_path(&path, column)
                .ok_or_else(|| anyhow!("{} is not a task", path.display()))?;
            // Find it in whichever column it currently sits in.
            let found = target.board.find(&task.stem)?;
            if found.column != column {
                // No UI escape hatch for a WIP limit in this pass — a
                // blocked drag just surfaces the refusal below, the same
                // treatment a lease refusal already gets.
                target.board.move_task(&found, column, false)?;
                // Dragging a card into Todo or Doing means "I'm starting
                // this" — claim the lease at the same moment, best-effort:
                // a failure here must never undo the move that already
                // succeeded, only surface as a status note below.
                if matches!(column, Column::Todo | Column::Doing)
                    && let Ok(entered) = target.board.find(&task.stem)
                    && let Ok(hash) = entered.hash()
                {
                    let hours = crate::board_settings::load(&target.board).lease_hours();
                    if let Err(err) = entered.take_lease(hash, hours) {
                        lease_note = Some(format!("moved, but couldn't claim the lease: {err}"));
                    }
                }
            }
            // Re-read: moving renamed the file.
            let moved = target.board.find(&task.stem)?;
            let hash = moved.hash()?;
            moved.set_order(hash, rank)
        })();

        match outcome {
            Ok(()) => self.status = lease_note,
            Err(err) => self.status = Some(format!("{err}")),
        }
        self.reload();
    }
}

/// The backlog, split into the loose tickets and one group per epic folder.
/// The loose group comes first and is always present, so there is somewhere to
/// drop a ticket you want to take *out* of an epic.
fn read_groups(board: &Board) -> Result<Vec<(Option<Group>, Vec<Card>)>> {
    let all = board.tasks_in(Column::Backlog)?;
    let mut groups: Vec<(Option<Group>, Vec<Card>)> = vec![(None, Vec::new())];
    for group in board.groups()? {
        groups.push((Some(group), Vec::new()));
    }
    for task in all {
        let slot = groups
            .iter_mut()
            .find(|(group, _)| group.as_ref().map(|g| g.name.as_str()) == task.epic.as_deref());
        if let Some((_, cards)) = slot {
            cards.push(card_from(task));
        }
    }
    for (_, cards) in &mut groups {
        sort_cards(cards);
    }
    Ok(groups)
}

fn card_from(task: Task) -> Card {
    let rank = task
        .order()
        .unwrap_or_else(|| task.id.map(f64::from).unwrap_or(f64::MAX));
    Card {
        title: task.title(),
        id: task.id_display(),
        assignee: task.assignee(),
        points: task.points(),
        rank,
        task,
    }
}

fn sort_cards(cards: &mut [Card]) {
    cards.sort_by(|a, b| {
        a.rank
            .partial_cmp(&b.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Put a typed body into a rendered ticket, keeping its frontmatter.
///
/// An empty body leaves the template's own, so clearing the box gets you the
/// default ticket rather than an empty file that `write` would refuse.
fn splice_body(rendered: &str, body: &str) -> String {
    if body.trim().is_empty() {
        return rendered.to_string();
    }
    match crate::task::split_body(rendered) {
        Some((head, _)) => format!("{head}{body}"),
        // No frontmatter to keep: the template is not one we understand, so
        // don't pretend — write what it rendered.
        None => rendered.to_string(),
    }
}

fn read_lanes(board: &Board) -> Result<Vec<Vec<Card>>> {
    let mut lanes = Vec::new();
    for column in Column::ALL {
        let mut cards: Vec<Card> = board.tasks_in(column)?.into_iter().map(card_from).collect();
        sort_cards(&mut cards);
        lanes.push(cards);
    }
    Ok(lanes)
}

#[cfg(feature = "gui")]
impl eframe::App for BoardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Applied every frame rather than only on change: it is two cheap
        // setters, and it means the window is right on the first paint after
        // loading from the config file.
        let ctx = ui.ctx().clone();
        ctx.set_theme(match self.settings.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
        // Zoom rather than the text styles alone: it scales spacing and the
        // card widths with the text, so a large font does not overflow a
        // column sized for a standard one.
        ctx.set_zoom_factor(self.settings.font.scale());
        // After set_theme, which resets visuals to the theme's own
        // defaults: everything already built on `ui.visuals()` (card fills,
        // borders, the drop-target highlight) picks this up for free, no
        // per-widget changes needed. `Accent::Default` applies nothing,
        // leaving the theme's own colours exactly as before this existed.
        if let Some(color) = self.settings.accent.color() {
            // Both light and dark styles, not just whichever is active right
            // now — under `ThemePreference::System` the OS can switch either
            // way, and the accent should still hold when it does.
            ctx.all_styles_mut(|style| {
                style.visuals.selection.bg_fill = color;
                style.visuals.hyperlink_color = color;
            });
        }

        let mut pending: Option<Drop> = None;
        let mut reload = false;
        let mut opened: Option<PathBuf> = None;

        egui::Panel::top(egui::Id::new("repos")).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("agile-md");
                ui.separator();
                ui.selectable_value(&mut self.view, View::Board, "Board");
                ui.selectable_value(&mut self.view, View::Backlog, "Backlog");
                ui.separator();
                match self.view {
                    // The board stacks every repository; the backlog is long
                    // enough that one at a time reads better.
                    View::Board => {
                        for repo in &mut self.repos {
                            ui.checkbox(&mut repo.visible, &repo.name);
                        }
                    }
                    View::Backlog => {
                        if self.repos.len() > 1 {
                            for (index, repo) in self.repos.iter().enumerate() {
                                ui.selectable_value(&mut self.backlog_repo, index, &repo.name);
                            }
                        } else {
                            ui.weak(&self.repos[0].name);
                        }
                    }
                }
                ui.separator();
                // On both views, not just Backlog — an epic/sprint is filed
                // under backlog/ regardless of which view created it, the
                // same way "New ticket" already resolves its target repo
                // per-view below. Gating this to Backlog only made the
                // feature effectively invisible: the Board view is the
                // default, so these buttons were never on screen unless you
                // already knew to switch tabs first.
                for (kind, label) in [(Kind::Epic, "Add epic"), (Kind::Sprint, "Add sprint")] {
                    if ui.button(label).clicked() {
                        let repo = match self.view {
                            View::Backlog => self.backlog_repo,
                            View::Board => 0,
                        };
                        self.group_draft = Some(GroupDraft {
                            repo,
                            kind,
                            editing: None,
                            name: String::new(),
                            description: String::new(),
                            days: DEFAULT_DAYS.to_string(),
                            state: State::Pending,
                        });
                    }
                }
                if ui.button("New ticket").clicked() {
                    let repo = match self.view {
                        View::Backlog => self.backlog_repo,
                        View::Board => 0,
                    };
                    self.drafting = Some(Draft {
                        repo,
                        title: String::new(),
                        epic: None,
                        points: String::new(),
                        body: "\n## Notes\n\n\n## Acceptance criteria\n\n- [ ] \n".to_string(),
                        markdown: false,
                        line: None,
                    });
                }
                reload = ui.button("Reload").clicked();
                if ui.button("Settings").clicked() {
                    self.showing_settings = true;
                }
            });
            let mut all_tags = BTreeSet::new();
            for repo in &self.repos {
                for card in repo.lanes.iter().flatten() {
                    all_tags.extend(card.task.tags());
                }
            }
            if !all_tags.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.weak("Tags:");
                    for tag in &all_tags {
                        let mut on = self.tag_filter.contains(tag);
                        if ui.selectable_label(on, tag).clicked() {
                            on = !on;
                            if on {
                                self.tag_filter.insert(tag.clone());
                            } else {
                                self.tag_filter.remove(tag);
                            }
                        }
                    }
                    if !self.tag_filter.is_empty() && ui.small_button("clear").clicked() {
                        self.tag_filter.clear();
                    }
                });
            }
            if let Some(status) = &self.status {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), status);
            }
        });

        let mut refile: Option<(PathBuf, usize, Option<String>)> = None;
        let mut group_edit: Option<String> = None;
        let mut group_start: Option<String> = None;
        let mut group_empty: Option<String> = None;

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| match self.view {
                View::Board => {
                    for (repo_index, repo) in self.repos.iter().enumerate() {
                        if !repo.visible {
                            continue;
                        }
                        if self.repos.iter().filter(|r| r.visible).count() > 1 {
                            ui.label(egui::RichText::new(&repo.name).strong());
                        }
                        ui.horizontal_top(|ui| {
                            for column in Column::ALL {
                                if let Some(drop) = column_ui(
                                    ui,
                                    repo,
                                    repo_index,
                                    column,
                                    &self.tag_filter,
                                    self.identity.as_deref(),
                                    &mut opened,
                                ) {
                                    pending = Some(drop);
                                }
                            }
                        });
                        ui.add_space(12.0);
                    }
                }
                View::Backlog => {
                    let index = self.backlog_repo.min(self.repos.len() - 1);
                    let repo = &self.repos[index];
                    ui.horizontal_top(|ui| {
                        for (group, cards) in &repo.groups {
                            if let Some(target) = group_ui(
                                ui,
                                group.as_ref(),
                                cards,
                                index,
                                &self.tag_filter,
                                self.identity.as_deref(),
                                &mut GroupEvents {
                                    opened: &mut opened,
                                    edit: &mut group_edit,
                                    start: &mut group_start,
                                    empty: &mut group_empty,
                                },
                            ) {
                                refile = Some(target);
                            }
                        }
                    });
                }
            });
        });

        self.editor_ui(ui.ctx());
        self.draft_ui(ui.ctx());
        self.group_ui(ui.ctx());
        self.settings_ui(&ctx);

        if let Some(name) = group_edit {
            self.open_group(&name);
        }
        if let Some(name) = group_start {
            self.start_sprint(&name);
        }
        if let Some(name) = group_empty {
            self.empty_group_now(&name);
        }

        if let Some((path, repo, epic)) = refile {
            self.refile(&path, repo, epic);
        }

        if let Some(path) = opened {
            match self
                .repos
                .iter()
                .enumerate()
                .flat_map(|(index, repo)| repo.lanes.iter().flatten().map(move |c| (index, c)))
                .find(|(_, card)| card.task.path == path)
                .map(|(index, card)| Editor::open(index, card))
            {
                Some(Ok(editor)) => self.editing = Some(editor),
                Some(Err(err)) => self.status = Some(format!("{err}")),
                None => {}
            }
        }

        if let Some(drop) = pending {
            self.apply(drop);
        } else if reload {
            self.reload();
        }
    }
}

/// The settings overlay: theme and text size, both remembered.
#[cfg(feature = "gui")]
impl BoardApp {
    fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.showing_settings {
            return;
        }
        let before = self.settings;
        let mut close = false;

        let modal = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Settings");
            ui.separator();

            ui.label("Text size");
            ui.horizontal(|ui| {
                for size in [FontSize::Standard, FontSize::Medium, FontSize::Large] {
                    ui.selectable_value(&mut self.settings.font, size, size.label());
                }
            });
            ui.weak(format!("{}x the standard size", self.settings.font.scale()));

            ui.add_space(8.0);
            ui.label("Colour scheme");
            ui.horizontal(|ui| {
                for (theme, label) in [
                    (Theme::System, "System"),
                    (Theme::Light, "Light"),
                    (Theme::Dark, "Dark"),
                ] {
                    ui.selectable_value(&mut self.settings.theme, theme, label);
                }
            });
            ui.weak("remembered for next time");

            ui.add_space(8.0);
            ui.label("Accent colour");
            ui.horizontal_wrapped(|ui| {
                for accent in Accent::ALL {
                    ui.selectable_value(&mut self.settings.accent, accent, accent.label());
                }
            });
            ui.weak("applies over either colour scheme above");

            ui.separator();
            close = ui.button("Close").clicked();
        });

        // Saved on change rather than on close, so quitting the window without
        // pressing anything still keeps the choice.
        if self.settings != before
            && let Err(err) = self.settings.save()
        {
            self.status = Some(format!("{err}"));
        }
        if close || modal.should_close() {
            self.showing_settings = false;
        }
    }
}

/// Creating and editing an epic or a sprint. One overlay for both, because
/// they differ in two fields, not in kind of thing.
#[cfg(feature = "gui")]
impl BoardApp {
    fn open_group(&mut self, name: &str) {
        match self.repos[self.backlog_repo].board.group(name) {
            Ok(group) => {
                self.group_draft = Some(GroupDraft {
                    repo: self.backlog_repo,
                    kind: group.kind,
                    editing: Some(group.name.clone()),
                    name: group.name,
                    description: group.description,
                    days: group.days.to_string(),
                    state: group.state,
                });
            }
            Err(err) => self.status = Some(format!("{err}")),
        }
    }

    fn start_sprint(&mut self, name: &str) {
        let outcome = (|| -> Result<()> {
            let mut group = self.repos[self.backlog_repo].board.group(name)?;
            group.start()
        })();
        match outcome {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("{err}")),
        }
        self.reload();
    }

    /// Empty a group's folder into the loose backlog at once, best-effort:
    /// a ticket that couldn't move (someone else's active lease, say) is
    /// named in the status line rather than silently dropped or aborting
    /// the ones that did move.
    fn empty_group_now(&mut self, name: &str) {
        match self.repos[self.backlog_repo].board.empty_group(name) {
            Ok(result) if result.skipped.is_empty() => self.status = None,
            Ok(result) => {
                let names: Vec<&str> = result.skipped.iter().map(|(s, _)| s.as_str()).collect();
                self.status = Some(format!(
                    "moved {}, couldn't move {}",
                    result.moved.len(),
                    names.join(", ")
                ));
            }
            Err(err) => self.status = Some(format!("{err}")),
        }
        self.reload();
    }

    fn group_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.group_draft else {
            return;
        };

        let mut apply = false;
        let mut cancel = false;
        let creating = draft.editing.is_none();
        let is_sprint = draft.kind == Kind::Sprint;
        let started = draft.state == State::Started;
        let noun = draft.kind.as_str();

        let modal = egui::Modal::new(egui::Id::new("group-editor")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(if creating {
                format!("New {noun}")
            } else {
                format!("Edit {noun}")
            });
            ui.separator();

            egui::Grid::new("group-fields")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    // The name is the directory, so renaming would move every
                    // ticket in it and orphan the `epic` each one records.
                    ui.add_enabled(
                        creating,
                        egui::TextEdit::singleline(&mut draft.name)
                            .desired_width(f32::INFINITY)
                            .hint_text("checkout"),
                    );
                    ui.end_row();

                    ui.label("Description");
                    ui.add(
                        egui::TextEdit::multiline(&mut draft.description)
                            .desired_width(f32::INFINITY)
                            .desired_rows(3),
                    );
                    ui.end_row();

                    if is_sprint {
                        ui.label("Days");
                        ui.add_enabled(
                            !started,
                            egui::TextEdit::singleline(&mut draft.days).desired_width(60.0),
                        );
                        ui.end_row();

                        ui.label("State");
                        if started {
                            ui.label(egui::RichText::new("started — cannot be undone").strong());
                        } else {
                            ui.weak("pending (start it from the board)");
                        }
                        ui.end_row();
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                let named = !draft.name.trim().is_empty();
                apply = ui
                    .add_enabled(
                        named,
                        egui::Button::new(if creating { "Create" } else { "Save" }),
                    )
                    .clicked();
                cancel = ui.button("Cancel").clicked();
                if is_sprint && !started {
                    ui.weak("only sized tickets can go in a sprint");
                }
            });
        });

        if apply {
            self.save_group();
        } else if cancel || modal.should_close() {
            self.group_draft = None;
        }
    }

    fn save_group(&mut self) {
        let Some(draft) = &self.group_draft else {
            return;
        };
        let repo = draft.repo;
        let creating = draft.editing.is_none();
        let group = Group {
            dir: self.repos[repo]
                .board
                .dir(Column::Backlog)
                .join(draft.name.trim()),
            name: draft.name.trim().to_string(),
            kind: draft.kind,
            description: draft.description.trim().to_string(),
            days: draft.days.trim().parse().unwrap_or(DEFAULT_DAYS),
            state: draft.state,
        };

        let board = &self.repos[repo].board;
        let outcome = if creating {
            board.create_group(&group)
        } else {
            group.save()
        };

        match outcome {
            Ok(()) => {
                self.status = None;
                self.group_draft = None;
                self.reload();
            }
            Err(err) => self.status = Some(format!("{err}")),
        }
    }
}

/// The new-ticket overlay. Creating from the board is a deliberate act, so it
/// asks for the few things worth deciding up front and leaves the body to the
/// editor.
#[cfg(feature = "gui")]
impl BoardApp {
    fn draft_ui(&mut self, ctx: &egui::Context) {
        let Some(draft) = &mut self.drafting else {
            return;
        };

        let mut create = false;
        let mut cancel = false;
        // A sprint only takes sized tickets, so offering one here would create
        // a ticket the board then refuses to file. Epics take anything.
        let epics = self.repos[draft.repo]
            .groups
            .iter()
            .filter_map(|(group, _)| group.as_ref())
            .filter(|group| group.accepts_changes() && !group.is_sprint())
            .map(|group| group.name.clone())
            .collect::<Vec<_>>();
        let repo_name = self.repos[draft.repo].name.clone();

        let modal = egui::Modal::new(egui::Id::new("new-ticket")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(format!("New ticket in {repo_name}"));
            ui.separator();

            egui::Grid::new("draft")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Title");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.title).desired_width(f32::INFINITY),
                    );
                    ui.end_row();

                    ui.label("Epic");
                    egui::ComboBox::from_id_salt("epic")
                        .selected_text(draft.epic.clone().unwrap_or_else(|| "(none)".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.epic, None, "(none)");
                            for epic in &epics {
                                ui.selectable_value(&mut draft.epic, Some(epic.clone()), epic);
                            }
                        });
                    ui.end_row();

                    ui.label("Points");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.points)
                            .desired_width(80.0)
                            .hint_text("e.g. 3"),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Body");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if draft.markdown { "RT" } else { "MD" };
                    if ui.button(label).clicked() {
                        draft.markdown = !draft.markdown;
                        draft.line = None;
                    }
                });
            });
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    if draft.markdown {
                        ui.add(
                            egui::TextEdit::multiline(&mut draft.body)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(10),
                        );
                    } else {
                        richtext::ui(ui, &mut draft.body, &mut draft.line);
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                create = ui
                    .add_enabled(!draft.title.trim().is_empty(), egui::Button::new("Create"))
                    .clicked();
                cancel = ui.button("Cancel").clicked();
                ui.weak("lands in the backlog");
            });
        });

        if create {
            self.create_ticket();
        } else if cancel || modal.should_close() {
            self.drafting = None;
        }
    }

    /// Create the drafted ticket through the same path `amd new` uses, then
    /// file it under its epic and size it.
    fn create_ticket(&mut self) {
        let Some(draft) = &self.drafting else { return };
        let repo = draft.repo;
        let title = draft.title.trim().to_string();
        let epic = draft.epic.clone();
        let points = draft.points.trim().to_string();
        let body = draft.body.clone();

        let outcome = (|| -> Result<()> {
            let board = &self.repos[repo].board;
            let _ = &body;
            // The same two steps `amd new` takes: render a draft from the
            // templates, then write it. Going through create:: means a ticket
            // made here is identical to one made from the CLI.
            let templates = crate::templates::Templates::load(board)?;
            let draft = crate::create::Draft::prepare(
                board,
                &templates,
                crate::create::NewTicket {
                    title: title.clone(),
                    template: crate::templates::DEFAULT_TEMPLATE.to_string(),
                    branch_type: String::new(),
                    assignee: String::new(),
                    parent: None,
                    related: Vec::new(),
                    tags: Vec::new(),
                    extra: Default::default(),
                },
            )?;
            // write() takes the *whole file*, which is what `amd new` hands
            // it after the editor has been over the rendered ticket. Passing
            // the notes alone wrote a file with no frontmatter, and every
            // call after it then failed on a ticket that was not one.
            let text = splice_body(&draft.body, &body);
            let created = draft.write(board, Some(text))?;
            if !points.is_empty() {
                created.task.set_points(created.task.hash()?, &points)?;
            }
            board.set_epic(&created.task, epic.as_deref())?;
            Ok(())
        })();

        match outcome {
            Ok(()) => {
                self.status = None;
                self.drafting = None;
                self.reload();
            }
            Err(err) => self.status = Some(format!("{err}")),
        }
    }
}

/// The edit overlay: a modal over the board, dismissed with Escape or by
/// clicking outside, which egui reports through `should_close`.
#[cfg(feature = "gui")]
impl BoardApp {
    fn editor_ui(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.editing else {
            return;
        };

        let mut save = false;
        let mut cancel = false;
        let id = editor.id.clone();

        let modal = egui::Modal::new(egui::Id::new("ticket-editor")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(format!("Ticket {id}"));
            ui.separator();

            egui::Grid::new("fields")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Title");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.title).desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                    ui.label("Assignee");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.assignee)
                            .desired_width(f32::INFINITY)
                            .hint_text("unassigned"),
                    );
                    ui.end_row();
                    ui.label("Points");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.points)
                            .desired_width(80.0)
                            .hint_text("unsized"),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Body");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Top right, and it says what you get, not what you are in.
                    let label = if editor.markdown { "RT" } else { "MD" };
                    if ui
                        .button(label)
                        .on_hover_text(if editor.markdown {
                            "switch to rich text"
                        } else {
                            "switch to markdown"
                        })
                        .clicked()
                    {
                        editor.markdown = !editor.markdown;
                        editor.line = None;
                    }
                });
            });
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    if editor.markdown {
                        ui.add(
                            egui::TextEdit::multiline(&mut editor.body)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(16),
                        );
                    } else {
                        richtext::ui(ui, &mut editor.body, &mut editor.line);
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                cancel = ui.button("Cancel").clicked();
                ui.weak("the filename keeps its original slug");
            });
        });

        if save {
            self.save();
        } else if cancel || modal.should_close() {
            self.editing = None;
        }
    }
}

/// The bits of frame state `group_ui` reports back by writing into, grouped
/// so the function stays under clippy's argument-count lint rather than
/// taking `opened`/`edit`/`start` as three separate `&mut` parameters.
#[cfg(feature = "gui")]
struct GroupEvents<'a> {
    opened: &'a mut Option<PathBuf>,
    edit: &'a mut Option<String>,
    start: &'a mut Option<String>,
    empty: &'a mut Option<String>,
}

/// One epic group in the backlog: a column of cards under the epic's name,
/// with the loose backlog first. Dropping a card here files it under this
/// epic, which moves the file and rewrites its `epic` key to match.
#[cfg(feature = "gui")]
fn group_ui(
    ui: &mut egui::Ui,
    group: Option<&Group>,
    cards: &[Card],
    repo_index: usize,
    filter: &BTreeSet<String>,
    identity: Option<&str>,
    events: &mut GroupEvents,
) -> Option<(PathBuf, usize, Option<String>)> {
    let heading = group.map_or("(unfiled)", |group| group.name.as_str());
    // A group has no index-based drop slots the way a column does, so — unlike
    // `column_ui` — filtering the display list here carries no risk of
    // misplacing a drop; there is nothing keyed off position to misplace.
    let cards: Vec<&Card> = cards
        .iter()
        .filter(|card| card_visible(card, filter))
        .collect();
    // Only sized tickets count, and a sprint refuses unsized ones, so its
    // total is the whole of what it committed to — of the cards currently
    // shown, matching the visible count in the heading below.
    let points: i64 = cards
        .iter()
        .filter_map(|card| card.points.as_deref())
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .sum();

    let frame = egui::Frame::default()
        .inner_margin(6.0)
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(4.0);

    let (_, payload) = ui.dnd_drop_zone::<Dragged, ()>(frame, |ui| {
        ui.vertical(|ui| {
            ui.set_width(COLUMN_WIDTH);
            ui.set_min_height(160.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{heading}  ({})", cards.len()))
                        .monospace()
                        .strong(),
                );
                if let Some(group) = group {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("edit").clicked() {
                            *events.edit = Some(group.name.clone());
                        }
                        // No precondition, matching a single ticket's own
                        // "no precondition" move out of a group — including
                        // a started sprint, which already lets tickets out
                        // individually.
                        if !cards.is_empty() && ui.small_button("empty").clicked() {
                            *events.empty = Some(group.name.clone());
                        }
                    });
                }
            });

            if let Some(group) = group {
                if group.is_sprint() {
                    ui.horizontal(|ui| {
                        ui.weak(format!("sprint · {}d · {points} pts", group.days));
                        match group.state {
                            State::Pending => {
                                if ui.small_button("start").clicked() {
                                    *events.start = Some(group.name.clone());
                                }
                            }
                            // No control to undo it: the flag is one way.
                            State::Started => {
                                ui.label(egui::RichText::new("started").strong());
                            }
                        }
                    });
                } else if points > 0 {
                    ui.weak(format!("epic · {points} pts"));
                } else {
                    ui.weak("epic");
                }
                if !group.description.is_empty() {
                    ui.weak(&group.description);
                }
            }
            ui.separator();

            let empty = cards.is_empty();
            for card in cards {
                if card_ui(ui, card, repo_index, identity) {
                    *events.opened = Some(card.task.path.clone());
                }
            }
            if empty {
                ui.weak("(empty)");
            }
        });
    });

    payload.and_then(|dragged| {
        (dragged.repo == repo_index).then(|| {
            (
                dragged.path.clone(),
                repo_index,
                group.map(|group| group.name.clone()),
            )
        })
    })
}

/// One status column: a header, then its cards top to bottom, with a drop slot
/// above each card and one at the end. Columns sit side by side, so dragging a
/// card up or down reorders it and dragging it across changes its status.
#[cfg(feature = "gui")]
fn column_ui(
    ui: &mut egui::Ui,
    repo: &RepoBoard,
    repo_index: usize,
    column: Column,
    filter: &BTreeSet<String>,
    identity: Option<&str>,
    opened: &mut Option<PathBuf>,
) -> Option<Drop> {
    let mut result = None;
    let cards = &repo.lanes[column as usize];
    let visible = cards
        .iter()
        .filter(|card| card_visible(card, filter))
        .count();

    let frame = egui::Frame::default()
        .inner_margin(6.0)
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(4.0);

    // Slots keep their index into the *full* lane even when a tag filter
    // hides some of its cards — that index is also what `apply()` ranks
    // against, unfiltered. Filtering the display but not the index space
    // would place a drop between the wrong pair of real neighbours whenever
    // a hidden card sat between two visible ones.
    let (_, payload) = ui.dnd_drop_zone::<Dragged, ()>(frame, |ui| {
        ui.vertical(|ui| {
            ui.set_width(COLUMN_WIDTH);
            ui.set_min_height(160.0);
            ui.label(
                egui::RichText::new(format!("{}  ({visible})", column.as_str().to_uppercase()))
                    .monospace()
                    .strong(),
            );
            for (index, card) in cards.iter().enumerate() {
                if !card_visible(card, filter) {
                    continue;
                }
                if let Some(drop) = slot(ui, repo_index, column, index) {
                    result = Some(drop);
                }
                if card_ui(ui, card, repo_index, identity) {
                    *opened = Some(card.task.path.clone());
                }
            }
            if let Some(drop) = slot(ui, repo_index, column, cards.len()) {
                result = Some(drop);
            }
            if visible == 0 {
                ui.weak("(empty)");
            }
        });
    });

    // Dropped on the lane but not on a slot: append.
    if let Some(dragged) = payload
        && result.is_none()
        && dragged.repo == repo_index
    {
        result = Some(Drop::Land {
            path: dragged.path.clone(),
            repo: repo_index,
            column,
            index: cards.len(),
        });
    }
    result
}

/// A thin gap between cards that accepts a drop, giving exact placement
/// without measuring pointer geometry. Horizontal, since cards stack downwards.
#[cfg(feature = "gui")]
fn slot(ui: &mut egui::Ui, repo_index: usize, column: Column, index: usize) -> Option<Drop> {
    let frame = egui::Frame::default().inner_margin(2.0);
    let (_, payload) = ui.dnd_drop_zone::<Dragged, ()>(frame, |ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(COLUMN_WIDTH - 16.0, 6.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) && egui::DragAndDrop::has_any_payload(ui.ctx()) {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
        }
    });
    payload.and_then(|dragged| {
        // A ticket carries its repository's ids and branch names with it, so
        // dropping one on another board would need a new id, not a rename.
        (dragged.repo == repo_index).then(|| Drop::Land {
            path: dragged.path.clone(),
            repo: repo_index,
            column,
            index,
        })
    })
}

/// A ticket: the width of its column, as tall as its title needs, so a column
/// reads as a stack.
#[cfg(feature = "gui")]
fn card_ui(ui: &mut egui::Ui, card: &Card, repo_index: usize, identity: Option<&str>) -> bool {
    let id = egui::Id::new(("card", &card.task.path));
    let payload = Dragged {
        path: card.task.path.clone(),
        repo: repo_index,
    };
    // Someone else's active lease gets a visible, attention-colour badge —
    // not just a refusal on save — so a conflict is avoidable rather than
    // only discovered after the fact (docs/adr/0007-concurrency-and-locking.md's
    // own stated cost: "an expired lease someone still believes in is a
    // surprise"). Your own lease gets a quieter marker, mostly so a card you
    // hold reads differently from one nobody's touched.
    let lease_badge = card
        .task
        .lease_active()
        .then(|| card.task.locked_by())
        .flatten();
    let dragged = ui.dnd_drag_source(id, payload, |ui| {
        egui::Frame::default()
            .inner_margin(8.0)
            .fill(ui.visuals().panel_fill)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.set_width(COLUMN_WIDTH - 28.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&card.id).monospace().weak());
                        if let Some(who) = &card.assignee {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.weak(who);
                                },
                            );
                        }
                    });
                    ui.label(&card.title);
                    if let Some(holder) = &lease_badge {
                        let mine = identity.is_some_and(|me| me == holder);
                        let text = if mine {
                            egui::RichText::new(format!("🔒 {holder}")).weak()
                        } else {
                            egui::RichText::new(format!("🔒 {holder}"))
                                .color(egui::Color32::from_rgb(200, 140, 40))
                                .strong()
                        };
                        ui.label(text);
                    }
                });
            });
    });

    // dnd_drag_source senses drag only, so the double-click needs its own
    // interaction over the same rectangle.
    ui.interact(dragged.response.rect, id.with("open"), egui::Sense::click())
        .double_clicked()
}

#[cfg(feature = "gui")]
pub fn run() -> Result<()> {
    let app = BoardApp::load()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "agile-md",
        options,
        Box::new(|_cc| Ok(Box::new(app) as Box<dyn eframe::App>)),
    )
    .map_err(|err| anyhow!("{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDERED: &str = "---\nid: \"001\"\ntitle: \"T\"\npoints: \"\"\n---\n\n## Notes\n\n\n";

    #[test]
    fn a_typed_body_keeps_the_rendered_frontmatter() {
        // The bug this guards: passing the notes alone to Draft::write wrote a
        // file with no frontmatter, so every call after it — set_points,
        // set_epic — failed and the new-ticket overlay could never close.
        let spliced = splice_body(RENDERED, "\n## Notes\n\nwhat I typed\n");
        assert!(
            spliced.starts_with("---\nid: \"001\"\n"),
            "frontmatter must survive: {spliced}"
        );
        assert!(spliced.ends_with("what I typed\n"), "{spliced}");
        assert_eq!(
            spliced.matches("---").count(),
            2,
            "and must not be duplicated: {spliced}"
        );
    }

    #[test]
    fn an_empty_body_falls_back_to_the_template() {
        // write() refuses an empty ticket, so clearing the box must give the
        // default rather than nothing.
        assert_eq!(splice_body(RENDERED, "   \n"), RENDERED);
        assert_eq!(splice_body(RENDERED, ""), RENDERED);
    }

    #[test]
    fn a_template_without_frontmatter_is_left_as_rendered() {
        let odd = "no frontmatter at all\n";
        assert_eq!(splice_body(odd, "typed"), odd);
    }

    fn card(dir: &std::path::Path, name: &str, body: &str) -> Card {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        card_from(Task::from_path(&path, Column::Todo).unwrap())
    }

    #[test]
    fn card_visible_matches_any_tag_in_the_filter() {
        let dir = tempfile::tempdir().unwrap();
        let tagged = card(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"A\"\ntags: [urgent]\n---\n\nbody\n",
        );
        let untagged = card(
            dir.path(),
            "002-b.md",
            "---\nid: \"002\"\ntitle: \"B\"\n---\n\nbody\n",
        );

        let empty = BTreeSet::new();
        assert!(
            card_visible(&tagged, &empty) && card_visible(&untagged, &empty),
            "an empty filter matches everything"
        );

        let mut filter = BTreeSet::new();
        filter.insert("urgent".to_string());
        assert!(card_visible(&tagged, &filter));
        assert!(!card_visible(&untagged, &filter));

        filter.insert("other".to_string());
        assert!(
            card_visible(&tagged, &filter),
            "matches any tag in the filter (OR), not all of them"
        );
    }
}
