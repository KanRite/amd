//! The new-ticket / edit-ticket form: state and key handling, no drawing.
//!
//! Every editable field of a ticket, then the body. The fields are single
//! lines with a real cursor; the body is edited a line at a time with the
//! same rules as the desktop board (`agile_md::richtext`) — Enter on a list
//! item opens the next one with the same marker, `Ctrl-T` flips a checkbox —
//! and what is saved is exactly the lines typed, never a re-rendering.

use agile_md::branch;
use agile_md::richtext::{self, Kind};
use agile_md::task::{ContentHash, Task, VALID_POINTS, parse_list};
use agile_md::transition::TicketFields;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What saving the form does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A new ticket in repository `repo`.
    New { repo: usize },
    /// The ticket with this file stem in repository `repo`, checked against
    /// the fingerprint taken when the form opened.
    Edit {
        repo: usize,
        stem: String,
        hash: ContentHash,
    },
}

/// A single line of text with a cursor, counted in characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    /// Character index of the cursor, `0..=text.chars().count()`.
    pub cursor: usize,
}

impl Line {
    pub fn new(text: &str) -> Self {
        Line {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Byte offset of a character index.
    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(at, _)| at)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor. False when there was none.
    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let at = self.byte(self.cursor - 1);
        self.text.remove(at);
        self.cursor -= 1;
        true
    }

    fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    /// Edit keys common to every text line. True when the key was one.
    fn key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace => {
                self.backspace();
            }
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }
}

/// A field that picks one of a fixed list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub options: Vec<String>,
    pub at: usize,
}

impl Choice {
    fn new(options: Vec<String>, current: &str) -> Self {
        let at = options.iter().position(|o| o == current).unwrap_or(0);
        Choice { options, at }
    }

    fn step(&mut self, forward: bool) {
        let len = self.options.len().max(1);
        self.at = if forward {
            (self.at + 1) % len
        } else {
            (self.at + len - 1) % len
        };
    }

    pub fn value(&self) -> &str {
        self.options.get(self.at).map_or("", String::as_str)
    }
}

/// How a form field is edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Text(Line),
    Pick(Choice),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub input: Input,
    pub hint: &'static str,
}

/// The "no value" entry of a choice, shown as written.
pub const NONE: &str = "(none)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub target: Target,
    pub fields: Vec<Field>,
    /// The body, one entry per line.
    pub body: Vec<Line>,
    /// Which line of the body the cursor is on.
    pub row: usize,
    /// `fields.len()` means the body has the focus.
    pub focus: usize,
    /// Why the last save failed. Cleared by the next key.
    pub error: Option<String>,
}

/// What a key did to the form, for the app to act on.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Editing,
    Save,
    Cancel,
}

const TITLE: usize = 0;
const ASSIGNEE: usize = 1;
const POINTS: usize = 2;
const BRANCH: usize = 3;
const TAGS: usize = 4;
const PARENT: usize = 5;
const RELATED: usize = 6;
const EPIC: usize = 7;

/// A fresh body with one empty line under each heading, like the desktop
/// board's — one place to start typing in each section, not a wall of blanks.
const NEW_BODY: &str = "## Notes\n\n## Acceptance criteria\n- [ ] ";

impl Form {
    /// A blank form for a new ticket. `groups` are the repository's epics and
    /// sprints, offered in the epic field.
    pub fn new_ticket(repo: usize, groups: &[String], epic: Option<&str>) -> Self {
        Self::build(
            Target::New { repo },
            FormValues {
                body: NEW_BODY.to_string(),
                epic: epic.unwrap_or("").to_string(),
                ..FormValues::default()
            },
            groups,
        )
    }

    /// A form filled from an existing ticket.
    pub fn edit_ticket(
        repo: usize,
        task: &Task,
        groups: &[String],
        hash: ContentHash,
        body: String,
    ) -> Self {
        let epic = if task.column == agile_md::board::Column::Backlog {
            task.epic.clone()
        } else {
            task.epic_meta()
        };
        Self::build(
            Target::Edit {
                repo,
                stem: task.stem.clone(),
                hash,
            },
            FormValues {
                title: task.title(),
                assignee: task.assignee().unwrap_or_default(),
                points: task.points().unwrap_or_default(),
                branch_type: task.branch_type().unwrap_or_default(),
                tags: task.tags().join(", "),
                parent: task.parent().unwrap_or_default(),
                related: task.related().join(", "),
                epic: epic.unwrap_or_default(),
                body,
            },
            groups,
        )
    }

    fn build(target: Target, values: FormValues, groups: &[String]) -> Self {
        let mut points = vec![NONE.to_string()];
        points.extend(VALID_POINTS.iter().map(|p| p.to_string()));
        let mut branch_types = vec![NONE.to_string()];
        branch_types.extend(branch::branch_types());
        let mut epics = vec![NONE.to_string()];
        epics.extend(groups.iter().cloned());
        let current = |value: &str| {
            if value.is_empty() {
                NONE.to_string()
            } else {
                value.to_string()
            }
        };
        let fields = vec![
            Field {
                label: "Title",
                input: Input::Text(Line::new(&values.title)),
                hint: "",
            },
            Field {
                label: "Assignee",
                input: Input::Text(Line::new(&values.assignee)),
                hint: "who is doing it; empty for nobody",
            },
            Field {
                label: "Points",
                input: Input::Pick(Choice::new(points, &current(&values.points))),
                hint: "←/→ to change",
            },
            Field {
                label: "Branch type",
                input: Input::Pick(Choice::new(branch_types, &current(&values.branch_type))),
                hint: "←/→ to change; none means no branch",
            },
            Field {
                label: "Tags",
                input: Input::Text(Line::new(&values.tags)),
                hint: "comma-separated",
            },
            Field {
                label: "Parent",
                input: Input::Text(Line::new(&values.parent)),
                hint: "an id or slug",
            },
            Field {
                label: "Related",
                input: Input::Text(Line::new(&values.related)),
                hint: "comma-separated ids or slugs",
            },
            Field {
                label: "Epic / sprint",
                input: Input::Pick(Choice::new(epics, &current(&values.epic))),
                hint: "←/→ to change",
            },
        ];
        let body: Vec<Line> = values.body.split('\n').map(Line::new).collect();
        let last = body.len().saturating_sub(1);
        Form {
            target,
            fields,
            body,
            row: last,
            focus: TITLE,
            error: None,
        }
    }

    /// Whether the body, not a field, has the focus.
    pub fn in_body(&self) -> bool {
        self.focus >= self.fields.len()
    }

    /// The form as the library wants it.
    pub fn to_fields(&self) -> TicketFields {
        let text = |at: usize| match &self.fields[at].input {
            Input::Text(line) => line.text.trim().to_string(),
            Input::Pick(_) => String::new(),
        };
        let pick = |at: usize| match &self.fields[at].input {
            Input::Pick(choice) if choice.value() != NONE => choice.value().to_string(),
            _ => String::new(),
        };
        let epic = Some(pick(EPIC)).filter(|epic| !epic.is_empty());
        TicketFields {
            title: text(TITLE),
            assignee: text(ASSIGNEE),
            points: pick(POINTS),
            epic,
            tags: parse_list(&text(TAGS)),
            branch_type: pick(BRANCH),
            parent: text(PARENT),
            related: parse_list(&text(RELATED)),
            body: self
                .body
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        self.error = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Char('s') if ctrl => return Outcome::Save,
            KeyCode::Tab => self.focus = (self.focus + 1) % (self.fields.len() + 1),
            KeyCode::BackTab => {
                self.focus = (self.focus + self.fields.len()) % (self.fields.len() + 1);
            }
            _ if self.in_body() => self.body_key(key, ctrl),
            _ => self.field_key(key.code),
        }
        Outcome::Editing
    }

    fn field_key(&mut self, code: KeyCode) {
        let last = self.fields.len();
        match &mut self.fields[self.focus].input {
            Input::Text(line) => {
                if !line.key(code) {
                    match code {
                        // Enter and Down both move on: a field is one line,
                        // so there is nothing for them to do inside it.
                        KeyCode::Enter | KeyCode::Down => self.focus = (self.focus + 1).min(last),
                        KeyCode::Up => self.focus = self.focus.saturating_sub(1),
                        _ => {}
                    }
                }
            }
            Input::Pick(choice) => match code {
                KeyCode::Left | KeyCode::Char('h') => choice.step(false),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => choice.step(true),
                KeyCode::Enter | KeyCode::Down | KeyCode::Char('j') => {
                    self.focus = (self.focus + 1).min(last);
                }
                KeyCode::Up | KeyCode::Char('k') => self.focus = self.focus.saturating_sub(1),
                _ => {}
            },
        }
    }

    fn body_key(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Char('t') if ctrl => {
                let line = &mut self.body[self.row];
                line.text = richtext::toggle(&line.text);
                line.cursor = line.cursor.min(line.len());
            }
            KeyCode::Enter => self.split_line(),
            KeyCode::Backspace => {
                if !self.body[self.row].backspace() {
                    self.join_with_previous();
                }
            }
            KeyCode::Up => {
                if self.row == 0 {
                    // Off the top of the body: back to the last field.
                    self.focus = self.fields.len() - 1;
                } else {
                    self.move_row(self.row - 1);
                }
            }
            KeyCode::Down => self.move_row((self.row + 1).min(self.body.len() - 1)),
            code => {
                self.body[self.row].key(code);
            }
        }
    }

    /// Move to another body line, keeping the column where it can be kept.
    fn move_row(&mut self, row: usize) {
        let column = self.body[self.row].cursor;
        self.row = row;
        let line = &mut self.body[row];
        line.cursor = column.min(line.len());
    }

    /// Enter: split the line at the cursor. On a list item the new line opens
    /// with the next marker; on an *empty* list item it ends the list instead,
    /// clearing the marker rather than stacking up empty items.
    fn split_line(&mut self) {
        let line = &mut self.body[self.row];
        let at_end = line.cursor == line.len();
        if at_end
            && matches!(
                richtext::classify(&line.text),
                Kind::Check { ref text, .. }
                    | Kind::Bullet { ref text, .. }
                    | Kind::Numbered { ref text, .. }
                    if text.trim().is_empty()
            )
        {
            line.text.clear();
            line.cursor = 0;
            return;
        }
        let split = line.byte(line.cursor);
        let rest = line.text.split_off(split);
        let prefix = if at_end {
            richtext::continuation(&line.text).unwrap_or_default()
        } else {
            String::new()
        };
        let mut next = Line::new(&format!("{prefix}{rest}"));
        next.cursor = prefix.chars().count();
        self.row += 1;
        self.body.insert(self.row, next);
    }

    /// Backspace at the start of a line: pull it up onto the one before.
    fn join_with_previous(&mut self) {
        if self.row == 0 {
            return;
        }
        let current = self.body.remove(self.row);
        self.row -= 1;
        let previous = &mut self.body[self.row];
        previous.cursor = previous.len();
        previous.text.push_str(&current.text);
    }
}

#[derive(Default)]
struct FormValues {
    title: String,
    assignee: String,
    points: String,
    branch_type: String,
    tags: String,
    parent: String,
    related: String,
    epic: String,
    body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(form: &mut Form, text: &str) {
        for c in text.chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn blank() -> Form {
        Form::new_ticket(0, &["GuIFix".to_string()], None)
    }

    fn body_of(form: &Form) -> Vec<&str> {
        form.body.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn a_new_form_starts_on_the_title_with_one_hint_per_section() {
        let form = blank();
        assert_eq!(form.focus, TITLE);
        assert_eq!(
            body_of(&form),
            ["## Notes", "", "## Acceptance criteria", "- [ ] "]
        );
    }

    #[test]
    fn typing_and_backspacing_edit_the_title_at_the_cursor() {
        let mut form = blank();
        type_text(&mut form, "Fix the thng");
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Left));
        type_text(&mut form, "i");
        assert_eq!(form.to_fields().title, "Fix the thing");
        form.handle_key(key(KeyCode::Backspace));
        assert_eq!(form.to_fields().title, "Fix the thng");
    }

    #[test]
    fn editing_handles_characters_that_are_wider_than_a_byte() {
        let mut form = blank();
        type_text(&mut form, "café");
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Backspace));
        assert_eq!(form.to_fields().title, "caé");
    }

    #[test]
    fn choices_cycle_and_none_means_empty() {
        let mut form = blank();
        form.focus = POINTS;
        assert_eq!(form.to_fields().points, "");
        form.handle_key(key(KeyCode::Right));
        assert_eq!(form.to_fields().points, "1");
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Left));
        assert_eq!(form.to_fields().points, "13", "wraps backwards past none");
        form.focus = EPIC;
        form.handle_key(key(KeyCode::Right));
        assert_eq!(form.to_fields().epic.as_deref(), Some("GuIFix"));
    }

    #[test]
    fn tags_and_related_split_on_commas() {
        let mut form = blank();
        form.focus = TAGS;
        type_text(&mut form, "ux, cli");
        form.focus = RELATED;
        type_text(&mut form, "12,13");
        let fields = form.to_fields();
        assert_eq!(fields.tags, ["ux", "cli"]);
        assert_eq!(fields.related, ["12", "13"]);
    }

    #[test]
    fn tab_walks_the_fields_then_the_body_then_round_again() {
        let mut form = blank();
        for _ in 0..form.fields.len() {
            form.handle_key(key(KeyCode::Tab));
        }
        assert!(form.in_body());
        form.handle_key(key(KeyCode::Tab));
        assert_eq!(form.focus, TITLE);
        form.handle_key(key(KeyCode::BackTab));
        assert!(form.in_body());
    }

    #[test]
    fn enter_on_a_checklist_item_opens_the_next_one() {
        let mut form = blank();
        form.focus = form.fields.len();
        type_text(&mut form, "write it");
        form.handle_key(key(KeyCode::Enter));
        type_text(&mut form, "test it");
        assert_eq!(&body_of(&form)[3..], ["- [ ] write it", "- [ ] test it"]);
    }

    #[test]
    fn enter_on_an_empty_list_item_ends_the_list() {
        let mut form = blank();
        form.focus = form.fields.len();
        form.handle_key(key(KeyCode::Enter));
        assert_eq!(body_of(&form).len(), 4, "no new line");
        assert_eq!(body_of(&form)[3], "");
    }

    #[test]
    fn enter_in_the_middle_of_a_line_splits_it_without_a_marker() {
        let mut form = blank();
        form.focus = form.fields.len();
        form.row = 0;
        form.body[0].cursor = 2;
        form.handle_key(key(KeyCode::Enter));
        assert_eq!(&body_of(&form)[..2], ["##", " Notes"]);
    }

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_to_the_one_above() {
        let mut form = blank();
        form.focus = form.fields.len();
        form.row = 2;
        form.body[2].cursor = 0;
        form.handle_key(key(KeyCode::Backspace));
        assert_eq!(body_of(&form)[1], "## Acceptance criteria");
        assert_eq!(form.row, 1);
    }

    #[test]
    fn ctrl_t_flips_a_checkbox() {
        let mut form = blank();
        form.focus = form.fields.len();
        type_text(&mut form, "done thing");
        form.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(body_of(&form)[3], "- [x] done thing");
    }

    #[test]
    fn up_from_the_top_of_the_body_returns_to_the_last_field() {
        let mut form = blank();
        form.focus = form.fields.len();
        form.row = 0;
        form.handle_key(key(KeyCode::Up));
        assert_eq!(form.focus, EPIC);
    }

    #[test]
    fn escape_cancels_and_ctrl_s_saves() {
        let mut form = blank();
        assert_eq!(form.handle_key(key(KeyCode::Esc)), Outcome::Cancel);
        assert_eq!(
            form.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Outcome::Save
        );
    }

    #[test]
    fn the_body_round_trips_exactly_as_typed() {
        let form = Form::new_ticket(0, &[], None);
        assert_eq!(
            form.to_fields().body,
            "## Notes\n\n## Acceptance criteria\n- [ ] "
        );
    }
}
