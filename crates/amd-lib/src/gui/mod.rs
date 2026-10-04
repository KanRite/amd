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

use crate::board::Column;
use crate::group::{DEFAULT_DAYS, Group, Kind, State};
use crate::model::{
    Card, RepoBoard, Repos, card_in_group, card_visible, load_repos, read_groups, read_lanes,
    splice_body,
};
use crate::task::{ContentHash, Task, VALID_POINTS};
use settings::{Accent, FontSize, GuiSettings, Theme};

/// Wide enough for a title of a few words without wrapping to three lines,
/// narrow enough that three columns and a scrollbar fit a laptop screen.
const COLUMN_WIDTH: f32 = 260.0;

/// The New-ticket dialog's starting body: one blank line to write notes on,
/// then a checklist item to write the first acceptance criterion on — one
/// empty click target per section, not two stacked ones (ticket 072, found
/// through the rich-text editor's own accessibility labels: two identical
/// "click to write…" hints back to back reads as a glitch, not an invitation).
const DEFAULT_DRAFT_BODY: &str = "## Notes\n\n## Acceptance criteria\n- [ ] ";

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
    assignee: String,
    points: String,
    /// Comma-separated; parsed with `task::parse_list` on create, same as a
    /// hand-typed `tags:` frontmatter line would be.
    tags: String,
    /// `branch::NONE` for no branch, else one of `branch::branch_types()`.
    branch_type: String,
    /// A ref (id or slug); empty for no parent.
    parent: String,
    /// Comma-separated refs.
    related: String,
    /// Notes written while creating, appended to the template's body.
    body: String,
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
    /// The epic/sprint this ticket is filed under, if any — a different,
    /// adjacent mechanism from the rest of these fields: saving goes through
    /// `Board::set_epic` (which can move the file), not `Task::apply_edit`,
    /// the same call the board view's own membership checkbox already makes.
    epic: Option<String>,
    /// Comma-separated; parsed with `task::parse_list` on save.
    tags: String,
    /// `branch::NONE` for no branch, else one of `branch::branch_types()`.
    branch_type: String,
    /// A ref (id or slug); empty for no parent.
    parent: String,
    /// Comma-separated refs.
    related: String,
    body: String,
    /// Fingerprint of the ticket's content when the editor opened. Saving
    /// checks this before writing — see `Task::apply_edit` and
    /// `docs/adr/0007-concurrency-and-locking.md`. A conflict means the
    /// ticket changed elsewhere while this was open; the fields above are
    /// stale and saving them would silently discard whatever changed it.
    hash: ContentHash,
    /// Which line the rich-text editor has open, kept between frames.
    line: richtext::Editing,
}

impl Editor {
    fn open(repo: usize, card: &Card) -> Result<Self> {
        // The folder decides inside `backlog/`; outside it there's no group
        // directory, so the frontmatter key is the only carrier — the same
        // split `card_in_group` makes.
        let epic = if card.task.column == Column::Backlog {
            card.task.epic.clone()
        } else {
            card.task.epic_meta()
        };
        Ok(Self {
            repo,
            path: card.task.path.clone(),
            id: card.id.clone(),
            title: card.task.title(),
            assignee: card.task.assignee().unwrap_or_default(),
            points: card.task.points().unwrap_or_default(),
            epic,
            tags: card.task.tags().join(","),
            branch_type: card.task.branch_type().unwrap_or_default(),
            parent: card.task.parent().unwrap_or_default(),
            related: card.task.related().join(","),
            body: card.task.body()?,
            hash: card.task.hash()?,
            line: None,
        })
    }
}

pub struct BoardApp {
    repos: Vec<RepoBoard>,
    /// Names the current project declares (`board_settings::repos()`) with
    /// no matching, board-having entry in the local `registry.rs` — known to
    /// exist, not cloned (or not registered) on this machine. Rendered
    /// greyed out and inert in the `View::Board` repo row, not omitted.
    unregistered: Vec<String>,
    status: Option<String>,
    editing: Option<Editor>,
    drafting: Option<Draft>,
    group_draft: Option<GroupDraft>,
    view: View,
    settings: GuiSettings,
    /// Open while the settings overlay is up.
    showing_settings: bool,
    /// Which repository the backlog view is showing, and which repository
    /// "New ticket"/"Add epic" target from the board view (which has no
    /// repo selector of its own — it stacks every repository rather than
    /// showing one at a time). Always `0` — `repos[0]` is the current
    /// project's own board, pushed first in `load()` by construction, not
    /// found by position in some other ordering.
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
    /// The WIP-limit text fields in the Settings modal — `backlog_repo`'s
    /// board-level `.settings`, not `GuiSettings` (personal/local). Loaded
    /// once when the modal opens (the "Settings" button handler), same
    /// lifecycle `GroupDraft` already uses for its own fields, empty string
    /// meaning unlimited either way.
    wip_todo_input: String,
    wip_doing_input: String,
}

impl BoardApp {
    pub fn load() -> Result<Self> {
        let Repos {
            repos,
            unregistered,
            identity,
        } = load_repos()?;
        // Always 0: `repos[0]` is `current`, pushed first above by
        // construction, not found by position in an alphabetically-sorted
        // list the way the old cross-project registry required.
        let backlog_repo = 0;
        Ok(Self {
            repos,
            unregistered,
            status: None,
            editing: None,
            drafting: None,
            group_draft: None,
            view: View::Board,
            settings: GuiSettings::load(),
            showing_settings: false,
            backlog_repo,
            tag_filter: BTreeSet::new(),
            identity,
            wip_todo_input: String::new(),
            wip_doing_input: String::new(),
        })
    }

    fn reload(&mut self) {
        // Deliberately doesn't re-derive `unregistered` — a repo newly
        // registered (or un-registered) elsewhere while `amdui` is already
        // running only takes effect on the next launch, same as any other
        // change to which repos exist wasn't live-tracked before this
        // either. `Reload` refreshes each already-open board's own content.
        for repo in &mut self.repos {
            match (read_lanes(&repo.board), read_groups(&repo.board)) {
                (Ok(lanes), Ok(groups)) => {
                    repo.lanes = lanes;
                    repo.groups = groups;
                }
                (Err(err), _) | (_, Err(err)) => self.status = Some(format!("{err}")),
            }
            // A selected group that's just been deleted (or emptied and
            // deleted elsewhere, or renamed by hand) shouldn't leave the
            // filter pointing at nothing — fall back to unfiltered rather
            // than a dangling name nothing will ever match again.
            if let Some(name) = &repo.selected_group
                && !repo
                    .groups
                    .iter()
                    .any(|(group, _)| group.as_ref().is_some_and(|g| &g.name == name))
            {
                repo.selected_group = None;
            }
        }
    }

    /// Set or clear a ticket's membership in a group, from its membership
    /// checkbox — `member: true` files it under `name`, `false` clears it.
    /// Works from any column: `Board::set_epic` only moves the file inside
    /// `backlog/`, elsewhere it tags the frontmatter in place.
    fn toggle_membership(&mut self, path: &std::path::Path, repo: usize, name: &str, member: bool) {
        let outcome = (|| -> Result<()> {
            let board = &self.repos[repo].board;
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| anyhow!("{} is not a task", path.display()))?;
            let found = board.find(stem)?;
            board.set_epic(&found, member.then_some(name))?;
            Ok(())
        })();
        match outcome {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("{err}")),
        }
        self.reload();
    }

    /// Write the editor's fields back to the ticket in one call
    /// (`Task::apply_edit`) — not several separate setters, because the hash
    /// captured when the editor opened can only ever match the file once.
    /// A conflict here means the ticket changed elsewhere while this was
    /// open; the status line reports it rather than silently overwriting
    /// whatever changed it. `epic` is applied second, through
    /// `Board::set_epic` — a different, adjacent action (it can move the
    /// file) from the frontmatter-only write above, the same call the board
    /// view's own membership checkbox already makes. It runs after, against
    /// whatever `apply_edit` just wrote, rather than against the stale
    /// captured hash.
    fn save(&mut self) {
        let Some(editor) = &self.editing else { return };
        let outcome = (|| -> Result<()> {
            let board = &self.repos[editor.repo].board;
            let task = self.repos[editor.repo]
                .lanes
                .iter()
                .flatten()
                .find(|card| card.task.path == editor.path)
                .map(|card| &card.task)
                .ok_or_else(|| anyhow!("{} is no longer on the board", editor.path.display()))?;
            let tags = crate::task::parse_list(&editor.tags);
            let related = crate::task::parse_list(&editor.related);
            task.apply_edit(
                editor.hash,
                board,
                crate::task::EditFields {
                    title: &editor.title,
                    assignee: &editor.assignee,
                    points: &editor.points,
                    tags: &tags,
                    branch_type: &editor.branch_type,
                    parent: &editor.parent,
                    related: &related,
                    body: &editor.body,
                },
            )?;
            board.set_epic(task, editor.epic.as_deref())?;
            Ok(())
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
        let rank = crate::model::rank_between(&lane, index);

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
        // After set_theme, which resets visuals to the theme's own defaults.
        // `selection.bg_fill`/`hyperlink_color` cover anything built from a
        // fill (card fills, the drop-target highlight), but a card border
        // and `ui.separator()` both paint `widgets.noninteractive.bg_stroke`
        // instead — a fill-only override never reaches them. `Accent::Default`
        // applies nothing, leaving the theme's own colours exactly as before
        // this existed.
        if let Some(color) = self.settings.accent.color() {
            // Both light and dark styles, not just whichever is active right
            // now — under `ThemePreference::System` the OS can switch either
            // way, and the accent should still hold when it does.
            ctx.all_styles_mut(|style| {
                style.visuals.selection.bg_fill = color;
                style.visuals.hyperlink_color = color;
                // `inactive`/`hovered`/`active` too, so a widget's outline
                // stays consistent across hover states rather than only at
                // rest.
                style.visuals.widgets.noninteractive.bg_stroke.color = color;
                style.visuals.widgets.inactive.bg_stroke.color = color;
                style.visuals.widgets.hovered.bg_stroke.color = color;
                style.visuals.widgets.active.bg_stroke.color = color;
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
                        // Declared by this project, not registered on this
                        // machine — visible so a teammate who's just added a
                        // repository knows it exists, inert because there's
                        // nothing local to show yet.
                        for name in &self.unregistered {
                            let mut moot = false;
                            ui.add_enabled(false, egui::Checkbox::new(&mut moot, name))
                                .on_disabled_hover_text(
                                    "declared for this project but not registered on this \
                                     machine — clone it, then `amd register`",
                                );
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
                // On both views, not just Backlog — picking a group here is
                // now also a board-wide filter (see column_ui/group lane
                // below), so it has to be reachable from the Board view too,
                // not only while looking at Backlog. Replaces the old
                // "Add epic"/"Add sprint" buttons: creating one now goes
                // through "New" here instead, targeting whichever kind the
                // toggle is on.
                {
                    let repo_index = self.backlog_repo;
                    let kind = self.repos[repo_index].selected_kind;
                    let mut new_kind = kind;
                    ui.selectable_value(&mut new_kind, Kind::Epic, "Epic");
                    ui.selectable_value(&mut new_kind, Kind::Sprint, "Sprint");
                    if new_kind != kind {
                        let repo = &mut self.repos[repo_index];
                        repo.selected_kind = new_kind;
                        // A name from the other kind's list has nothing to
                        // mean once the dropdown no longer lists that kind.
                        repo.selected_group = None;
                    }

                    let repo = &mut self.repos[repo_index];
                    let names: Vec<String> = repo
                        .groups
                        .iter()
                        .filter_map(|(group, _)| group.as_ref())
                        .filter(|group| group.kind == new_kind)
                        .map(|group| group.name.clone())
                        .collect();
                    egui::ComboBox::from_id_salt("group-filter")
                        .selected_text(
                            repo.selected_group
                                .clone()
                                .unwrap_or_else(|| "None".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut repo.selected_group, None, "None");
                            for name in &names {
                                ui.selectable_value(
                                    &mut repo.selected_group,
                                    Some(name.clone()),
                                    name,
                                );
                            }
                        });

                    if ui.button("New").clicked() {
                        self.group_draft = Some(GroupDraft {
                            repo: repo_index,
                            kind: new_kind,
                            editing: None,
                            name: String::new(),
                            description: String::new(),
                            days: DEFAULT_DAYS.to_string(),
                            state: State::Pending,
                        });
                    }

                    let selected = self.repos[repo_index].selected_group.clone();
                    if ui
                        .add_enabled(selected.is_some(), egui::Button::new("Delete"))
                        .clicked()
                        && let Some(name) = selected
                    {
                        self.delete_group_now(&name);
                    }
                }
                ui.separator();
                if ui.button("New ticket").clicked() {
                    // Board has no repo selector of its own — it always
                    // targets whichever repo the Backlog view would show
                    // (which starts out as the repo `amdui` was launched
                    // from).
                    let repo = self.backlog_repo;
                    self.drafting = Some(Draft {
                        repo,
                        title: String::new(),
                        epic: None,
                        assignee: String::new(),
                        points: String::new(),
                        tags: String::new(),
                        branch_type: crate::branch::NONE.to_string(),
                        parent: String::new(),
                        related: String::new(),
                        body: DEFAULT_DRAFT_BODY.to_string(),
                        line: None,
                    });
                }
                reload = ui.button("Reload").clicked();
                if ui.button("Settings").clicked() {
                    self.showing_settings = true;
                    // Board-level, not GuiSettings — load fresh from
                    // backlog_repo's own .settings each time the modal
                    // opens, same lifecycle GroupDraft's fields already use.
                    let board_settings =
                        crate::board_settings::load(&self.repos[self.backlog_repo].board);
                    self.wip_todo_input = board_settings
                        .wip_limit(Column::Todo)
                        .map(|limit| limit.to_string())
                        .unwrap_or_default();
                    self.wip_doing_input = board_settings
                        .wip_limit(Column::Doing)
                        .map(|limit| limit.to_string())
                        .unwrap_or_default();
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

        let mut membership_toggle: Option<(PathBuf, usize, String, bool)> = None;
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
                        let selected = repo.selected_group.as_deref();
                        ui.horizontal_top(|ui| {
                            for column in Column::ALL {
                                let mut toggled = None;
                                if let Some(drop) = column_ui(
                                    ui,
                                    repo,
                                    repo_index,
                                    column,
                                    &self.tag_filter,
                                    &mut ColumnEvents {
                                        identity: self.identity.as_deref(),
                                        selected,
                                        opened: &mut opened,
                                        toggled: &mut toggled,
                                    },
                                ) {
                                    pending = Some(drop);
                                }
                                if let (Some((path, member)), Some(name)) = (toggled, selected) {
                                    membership_toggle =
                                        Some((path, repo_index, name.to_string(), member));
                                }
                            }
                        });
                        ui.add_space(12.0);
                    }
                }
                View::Backlog => {
                    let index = self.backlog_repo.min(self.repos.len() - 1);
                    let repo = &self.repos[index];
                    let selected = repo.selected_group.as_deref();
                    // The selected group's own header (name, description,
                    // edit/start-if-sprint/empty) — its dedicated lane is
                    // gone, but that information still needs to live
                    // somewhere now that the backlog is one column, not one
                    // per group.
                    if let Some(name) = selected {
                        group_header_ui(
                            ui,
                            repo,
                            name,
                            &mut group_edit,
                            &mut group_start,
                            &mut group_empty,
                        );
                        ui.separator();
                    }
                    let mut toggled = None;
                    if let Some(drop) = column_ui(
                        ui,
                        repo,
                        index,
                        Column::Backlog,
                        &self.tag_filter,
                        &mut ColumnEvents {
                            identity: self.identity.as_deref(),
                            selected,
                            opened: &mut opened,
                            toggled: &mut toggled,
                        },
                    ) {
                        // Reordering within the loose backlog list — the
                        // same drag column_ui already supports for any
                        // column, unrelated to epic/sprint membership.
                        pending = Some(drop);
                    }
                    if let (Some((path, member)), Some(name)) = (toggled, selected) {
                        membership_toggle = Some((path, index, name.to_string(), member));
                    }
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

        if let Some((path, repo, name, member)) = membership_toggle {
            self.toggle_membership(&path, repo, &name, member);
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
        let wip_before = (self.wip_todo_input.clone(), self.wip_doing_input.clone());
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
            ui.label("WIP limits");
            egui::Grid::new("wip-limits")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Todo");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.wip_todo_input)
                            .desired_width(60.0)
                            .hint_text("none"),
                    );
                    ui.end_row();
                    ui.label("Doing");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.wip_doing_input)
                            .desired_width(60.0)
                            .hint_text("none"),
                    );
                    ui.end_row();
                });
            ui.weak("empty means unlimited — shared with this repository's board");

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
        let wip_after = (self.wip_todo_input.clone(), self.wip_doing_input.clone());
        if wip_after != wip_before {
            self.save_wip_limits();
        }
        if close || modal.should_close() {
            self.showing_settings = false;
        }
    }

    /// Board-level, unlike `self.settings` above — writes to
    /// `backlog_repo`'s own `.settings`, not the personal `GuiSettings`
    /// file. An empty or unparsable field parses to `None`, which
    /// `set_wip_limit` and `wip_limit()`'s read side already tolerate with
    /// no separate validation needed here — but the two fields no longer
    /// mean the same thing left blank: `wip_todo_input` blank still reads
    /// back as unlimited, while `wip_doing_input` blank now reads back as
    /// the column's own default of 1, not unlimited (`board_settings.rs`'s
    /// module doc explains why the two are asymmetric on purpose).
    fn save_wip_limits(&mut self) {
        let board = &self.repos[self.backlog_repo].board;
        let mut settings = crate::board_settings::load(board);
        let doing = self.wip_doing_input.trim().parse().ok();
        // Only doing is limited now (ticket 083); the todo field is
        // ignored until this board is replaced (ticket 084).
        let outcome = settings
            .set_wip_limit(Column::Doing, doing)
            .and_then(|()| settings.save(board));
        if let Err(err) = outcome {
            self.status = Some(format!("{err}"));
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

    /// Delete a group outright. Refuses (and leaves the selection alone) if
    /// it still has any tickets — `empty_group_now` above is the deliberate
    /// first step for that case, this doesn't do it implicitly.
    fn delete_group_now(&mut self, name: &str) {
        match self.repos[self.backlog_repo].board.delete_group(name) {
            Ok(()) => {
                self.status = None;
                self.repos[self.backlog_repo].selected_group = None;
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
                    points_combo(ui, &mut draft.points, "draft-points");
                    ui.end_row();

                    ui.label("Assignee");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.assignee)
                            .desired_width(f32::INFINITY)
                            .hint_text("unassigned"),
                    );
                    ui.end_row();

                    ui.label("Tags");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.tags)
                            .desired_width(f32::INFINITY)
                            .hint_text("comma-separated, e.g. gui, urgent"),
                    );
                    ui.end_row();

                    ui.label("Branch type");
                    branch_type_combo(ui, &mut draft.branch_type, "draft-branch-type");
                    ui.end_row();

                    ui.label("Parent");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.parent)
                            .desired_width(f32::INFINITY)
                            .hint_text("id or slug"),
                    );
                    ui.end_row();

                    ui.label("Related");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.related)
                            .desired_width(f32::INFINITY)
                            .hint_text("comma-separated ids or slugs"),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.label("Body");
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    richtext::ui(ui, &mut draft.body, &mut draft.line);
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
        let assignee = draft.assignee.trim().to_string();
        let points = draft.points.trim().to_string();
        let tags = crate::task::parse_list(&draft.tags);
        let branch_type = draft.branch_type.trim().to_string();
        let parent = draft.parent.trim().to_string();
        let related = crate::task::parse_list(&draft.related);
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
                    branch_type,
                    assignee,
                    parent: (!parent.is_empty()).then_some(parent),
                    related,
                    tags,
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
        // Same rule as the New ticket dialog: a sprint only takes sized
        // tickets, so it isn't offered here.
        let epics = self.repos[editor.repo]
            .groups
            .iter()
            .filter_map(|(group, _)| group.as_ref())
            .filter(|group| group.accepts_changes() && !group.is_sprint())
            .map(|group| group.name.clone())
            .collect::<Vec<_>>();

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
                    points_combo(ui, &mut editor.points, "editor-points");
                    ui.end_row();

                    ui.label("Epic");
                    egui::ComboBox::from_id_salt("editor-epic")
                        .selected_text(editor.epic.clone().unwrap_or_else(|| "(none)".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut editor.epic, None, "(none)");
                            for epic in &epics {
                                ui.selectable_value(&mut editor.epic, Some(epic.clone()), epic);
                            }
                        });
                    ui.end_row();

                    ui.label("Tags");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.tags)
                            .desired_width(f32::INFINITY)
                            .hint_text("comma-separated, e.g. gui, urgent"),
                    );
                    ui.end_row();

                    ui.label("Branch type");
                    branch_type_combo(ui, &mut editor.branch_type, "editor-branch-type");
                    ui.end_row();

                    ui.label("Parent");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.parent)
                            .desired_width(f32::INFINITY)
                            .hint_text("id or slug"),
                    );
                    ui.end_row();

                    ui.label("Related");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.related)
                            .desired_width(f32::INFINITY)
                            .hint_text("comma-separated ids or slugs"),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.label("Body");
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    richtext::ui(ui, &mut editor.body, &mut editor.line);
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

/// The selected group's own compact header — name, description, points (for
/// a sprint), and its edit/start/empty buttons. Replaces what used to be
/// each group's own lane header when every group got its own side-by-side
/// column; now there is one backlog list (`column_ui` on `Column::Backlog`)
/// and this renders just above it for whichever group is currently selected.
/// A drag-and-drop target to file a card into a specific group doesn't exist
/// any more — the membership checkbox on every card is what does that now,
/// and it works from any column, which a per-group drop lane never did.
#[cfg(feature = "gui")]
fn group_header_ui(
    ui: &mut egui::Ui,
    repo: &RepoBoard,
    name: &str,
    group_edit: &mut Option<String>,
    group_start: &mut Option<String>,
    group_empty: &mut Option<String>,
) {
    let Some(group) = repo
        .groups
        .iter()
        .filter_map(|(group, _)| group.as_ref())
        .find(|group| group.name == name)
    else {
        return;
    };
    let member_count = repo
        .lanes
        .iter()
        .flatten()
        .filter(|card| card_in_group(card, name))
        .count();
    let points: i64 = repo
        .lanes
        .iter()
        .flatten()
        .filter(|card| card_in_group(card, name))
        .filter_map(|card| card.points.as_deref())
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .sum();

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{name}  ({member_count})"))
                .monospace()
                .strong(),
        );
        if ui.small_button("edit").clicked() {
            *group_edit = Some(group.name.clone());
        }
        if member_count > 0 && ui.small_button("empty").clicked() {
            *group_empty = Some(group.name.clone());
        }
    });
    if group.is_sprint() {
        ui.horizontal(|ui| {
            ui.weak(format!("sprint · {}d · {points} pts", group.days));
            match group.state {
                State::Pending => {
                    if ui.small_button("start").clicked() {
                        *group_start = Some(group.name.clone());
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

/// The bits of frame state `column_ui` reads or reports back by writing
/// into, grouped so the function stays under clippy's argument-count lint —
/// the same reason `GroupEvents` used to exist here for the per-lane group
/// renderer this replaced.
#[cfg(feature = "gui")]
struct ColumnEvents<'a> {
    identity: Option<&'a str>,
    /// The currently selected epic/sprint, if any — `None` means no filter,
    /// draw plainly; `Some(name)` means every card gets a membership
    /// checkbox against `name`.
    selected: Option<&'a str>,
    opened: &'a mut Option<PathBuf>,
    /// A membership checkbox was toggled this frame: the card's path and its
    /// desired new membership state.
    toggled: &'a mut Option<(PathBuf, bool)>,
}

/// A dropdown of the board's accepted branch types, plus "no branch" —
/// `branch::choices()` already puts `NONE` first, so this is every valid
/// value `Task::apply_edit`/`create::Draft::prepare` will accept, offered
/// rather than typed so a typo can't slip through as an "unknown branch
/// type" error at save time.
#[cfg(feature = "gui")]
fn branch_type_combo(ui: &mut egui::Ui, value: &mut String, id_source: &str) {
    egui::ComboBox::from_id_salt(id_source)
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for choice in crate::branch::choices() {
                ui.selectable_value(value, choice.clone(), choice);
            }
        });
}

/// A dropdown of `task::VALID_POINTS`, plus "(none)" for unsized — the only
/// values `Task::set_points`/`apply_edit` actually accept, so this can never
/// produce a write the save call would then reject.
#[cfg(feature = "gui")]
fn points_combo(ui: &mut egui::Ui, points: &mut String, id_source: &str) {
    egui::ComboBox::from_id_salt(id_source)
        .selected_text(if points.is_empty() {
            "(none)"
        } else {
            points.as_str()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(points, String::new(), "(none)");
            for value in VALID_POINTS {
                ui.selectable_value(points, value.to_string(), value);
            }
        });
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
    events: &mut ColumnEvents,
) -> Option<Drop> {
    let cards = &repo.lanes[column as usize];
    let visible = cards
        .iter()
        .filter(|card| card_visible(card, filter))
        .count();

    let frame = egui::Frame::default()
        .inner_margin(6.0)
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(4.0);

    // `dnd_drop_zone` always overwrites *its own* frame's fill and stroke
    // with `widgets.inactive`/`active` — that's how it paints its own drag
    // highlight — so setting a stroke on `frame` above would be silently
    // discarded the moment a drag starts. An outer frame wrapping the drop
    // zone isn't touched by that override, so it's the only way to give the
    // column a border of our own choosing, all the time: with only a fill,
    // an empty column reads as a faint block with no edge at all, and two
    // columns side by side have nothing marking where one ends and the next
    // begins. Same stroke source as the ticket card's own border (`card_ui`,
    // below) — a non-default accent colours this too, for free (ticket 043).
    let outline = egui::Frame::default()
        .corner_radius(4.0)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke);

    let mut result = None;
    outline.show(ui, |ui| {
        // Slots keep their index into the *full* lane even when a tag filter
        // hides some of its cards — that index is also what `apply()` ranks
        // against, unfiltered. Filtering the display but not the index space
        // would place a drop between the wrong pair of real neighbours
        // whenever a hidden card sat between two visible ones.
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
                    let membership = events.selected.map(|name| card_in_group(card, name));
                    let response = card_ui(ui, card, repo_index, events.identity, membership);
                    if response.opened {
                        *events.opened = Some(card.task.path.clone());
                    }
                    if let Some(member) = response.toggled {
                        *events.toggled = Some((card.task.path.clone(), member));
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
    });
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

#[cfg(feature = "gui")]
struct CardResponse {
    /// The card was double-clicked to open it.
    opened: bool,
    /// The membership checkbox was clicked this frame; carries the desired
    /// new state, not the old one.
    toggled: Option<bool>,
}

/// A ticket: the width of its column, as tall as its title needs, so a column
/// reads as a stack. `membership` is `None` when no epic/sprint is selected
/// (draw plainly, no checkbox); `Some(is_member)` draws a checkbox and dims
/// the card when it isn't currently a member — dimmed rather than hidden,
/// specifically so a `todo`/`doing`/`done` card can still be *added* to the
/// selected group from wherever it already sits, not only removed.
#[cfg(feature = "gui")]
fn card_ui(
    ui: &mut egui::Ui,
    card: &Card,
    repo_index: usize,
    identity: Option<&str>,
    membership: Option<bool>,
) -> CardResponse {
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
    let mut toggled = None;
    let mut opened = false;
    ui.horizontal(|ui| {
        // The checkbox is a sibling of the draggable region, not nested
        // inside it — `dnd_drag_source` senses drag over its whole content,
        // and a checkbox nested inside it never got a clean click: it
        // started a drag instead, confirmed visually (the drop-zone
        // highlight lit up on a plain click). Keeping it outside means a
        // click here is an ordinary widget interaction, untouched by drag
        // sensing, while the card body to its right is still what you drag
        // to reorder or change column.
        //
        // Sibling placement alone wasn't enough, confirmed live: a bare,
        // unlabelled checkbox's native hit box is small (egui sizes it off
        // `interact_size`, not off any wrapper placed around it — wrapping
        // it in `add_sized` to force a bigger box was tried and confirmed
        // *not* to enlarge the actual clickable region, only the dead space
        // reserved around it, so a click there now hit neither the checkbox
        // nor the card). A short label turns the checkbox into a normal,
        // generously sized labelled control — the whole label is part of
        // the same clickable atom as the box in egui, which is what
        // actually gives a forgiving, unambiguous hit target.
        if let Some(is_member) = membership {
            let mut checked = is_member;
            if ui.checkbox(&mut checked, "in").changed() {
                toggled = Some(checked);
            }
            ui.add_space(6.0);
        }
        let dragged = ui.dnd_drag_source(id, payload, |ui| {
            // Opacity, not visibility — a non-member still needs to be
            // fully present and clickable for its checkbox to work.
            if membership == Some(false) {
                ui.set_opacity(0.5);
            }
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

        // dnd_drag_source senses drag only, so the double-click needs its
        // own interaction over the same rectangle.
        opened = ui
            .interact(dragged.response.rect, id.with("open"), egui::Sense::click())
            .double_clicked();
    });
    CardResponse { opened, toggled }
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
    use crate::model::card_from;

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

    /// Found live, right after ticket 071 fixed the blank-line click target:
    /// the New-ticket dialog's default body had *two* empty click targets
    /// back to back under each heading (two blank lines under "## Notes";
    /// a blank line then the empty checklist item under "## Acceptance
    /// criteria"), which reads as a glitch — identical "click to write…"
    /// hints stacked with nothing to tell them apart. `egui_kittest` counts
    /// the real rendered hints directly, so a future edit that reintroduces
    /// an extra blank line fails here instead of needing someone to notice
    /// it by eye in a live dialog again.
    #[test]
    fn the_default_draft_body_has_exactly_one_empty_hint_per_section() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let body = DEFAULT_DRAFT_BODY.to_string();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(500.0, 400.0))
            .build_ui_state(
                |ui, body: &mut String| {
                    let mut editing = None;
                    richtext::ui(ui, body, &mut editing);
                },
                body,
            );
        harness.run();

        let hints = harness.get_all_by_label_contains("click to write");
        assert_eq!(
            hints.count(),
            2,
            "one for the Notes blank line, one for the empty Acceptance \
             criteria checklist item — not a stacked duplicate of either"
        );
    }
}
