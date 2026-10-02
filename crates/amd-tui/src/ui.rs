//! Drawing. Reads `App`, never changes the board.

use agile_md::board::Column;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Mode, Section};
use crate::form::{Form, Input, Target};

const ACCENT: Color = Color::Cyan;

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(header_line(app), header);
    match &app.mode {
        Mode::Board | Mode::Help => board(frame, app, body),
        Mode::Ticket { lines, scroll } => ticket(frame, lines, *scroll, body),
        Mode::Filter { section, cursor } => filters(frame, app, *section, *cursor, body),
        Mode::Form(form) => form_screen(frame, app, form, body),
    }
    frame.render_widget(footer_line(app), footer);
    if app.mode == Mode::Help {
        help(frame, frame.area());
    }
}

fn header_line(app: &App) -> Paragraph<'static> {
    let names: Vec<&str> = app
        .repos
        .iter()
        .filter(|repo| repo.visible)
        .map(|repo| repo.name.as_str())
        .collect();
    let mut spans = vec![
        Span::styled(
            "agile-md",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {}", names.join(", "))),
    ];
    if !app.unregistered.is_empty() {
        spans.push(Span::styled(
            format!("  (not cloned: {})", app.unregistered.join(", ")),
            Style::new().add_modifier(Modifier::DIM),
        ));
    }
    let filter = filter_summary(app);
    if !filter.is_empty() {
        spans.push(Span::styled(
            format!("  filter: {filter}"),
            Style::new().fg(Color::Yellow),
        ));
    }
    Paragraph::new(Line::from(spans))
}

/// A one-line description of the active filters, empty when there are none.
pub fn filter_summary(app: &App) -> String {
    let mut parts = Vec::new();
    if !app.tags.is_empty() {
        let tags: Vec<&str> = app.tags.iter().map(String::as_str).collect();
        parts.push(format!("tags {}", tags.join("|")));
    }
    if let Some(group) = &app.group {
        parts.push(format!("group {group}"));
    }
    let hidden = app.repos.iter().filter(|repo| !repo.visible).count();
    if hidden > 0 {
        parts.push(format!("{hidden} repo(s) hidden"));
    }
    parts.join(" · ")
}

fn footer_line(app: &App) -> Paragraph<'static> {
    if let Mode::Form(form) = &app.mode {
        return match &form.error {
            Some(error) => Paragraph::new(error.clone()).style(Style::new().fg(Color::Red)),
            None => Paragraph::new(
                "Tab/Shift-Tab field  ←/→ cursor or choice  Ctrl-S save  Ctrl-T tick box  Esc cancel",
            )
            .style(Style::new().add_modifier(Modifier::DIM)),
        };
    }
    let text = match (&app.status, &app.mode) {
        (Some(status), _) => status.clone(),
        (None, Mode::Board) => {
            "h/l column  j/k ticket  H/L move  Enter open  n new  e edit  f filters  r reload  ? help  q quit"
                .to_string()
        }
        (None, Mode::Ticket { .. }) => "j/k scroll  Esc back".to_string(),
        (None, Mode::Filter { .. }) => {
            "Tab section  j/k move  Space toggle  c clear all  Esc back".to_string()
        }
        (None, Mode::Help) => "any key closes this".to_string(),
        (None, Mode::Form(_)) => String::new(),
    };
    let style = if app.status.is_some() {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };
    Paragraph::new(text).style(style)
}

fn board(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::horizontal(Column::ALL.map(|_| Constraint::Fill(1))).split(area);
    let several = app.several_repos();
    for (index, column) in Column::ALL.iter().enumerate() {
        let entries = app.visible(index);
        let items: Vec<ListItem> = entries
            .iter()
            .map(|entry| {
                let repo = &app.repos[entry.repo];
                let card = &repo.lanes[index][entry.card];
                let prefix = if several {
                    format!("{}/", repo.name)
                } else {
                    String::new()
                };
                let mut line = format!("{prefix}{} {}", card.id, card.title);
                if let Some(points) = card.points.as_deref().filter(|p| !p.is_empty()) {
                    line.push_str(&format!(" [{points}]"));
                }
                ListItem::new(line)
            })
            .collect();
        let selected_column = index == app.col;
        let border = if selected_column {
            Style::new().fg(ACCENT)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(border)
            .title(format!(
                " {} ({}) ",
                column.as_str().to_uppercase(),
                items.len()
            ));
        let list = List::new(items).block(block).highlight_style(
            Style::new()
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                .fg(if selected_column {
                    ACCENT
                } else {
                    Color::Reset
                }),
        );
        // Seed from last frame's offset and store back what the widget
        // decided, so the scroll position survives the redraw.
        let mut state = ListState::default()
            .with_offset(app.offsets[index].get())
            .with_selected(
                selected_column
                    .then_some(app.row)
                    .filter(|_| !entries.is_empty()),
            );
        frame.render_stateful_widget(list, columns[index], &mut state);
        app.offsets[index].set(state.offset());
    }
}

fn ticket(frame: &mut Frame, lines: &[String], scroll: u16, area: Rect) {
    let text: Vec<Line> = lines.iter().map(|line| Line::raw(line.as_str())).collect();
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(" ticket ");
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

fn filters(frame: &mut Frame, app: &App, section: Section, cursor: usize, area: Rect) {
    let [repos, tags, groups] = Layout::horizontal([Constraint::Fill(1); 3]).areas(area);

    let repo_rows: Vec<String> = app
        .repos
        .iter()
        .map(|repo| format!("{} {}", check(repo.visible), repo.name))
        .collect();
    let tag_rows: Vec<String> = app
        .all_tags()
        .into_iter()
        .map(|tag| format!("{} {tag}", check(app.tags.contains(&tag))))
        .collect();
    let mut group_rows = vec![format!("{} (no group filter)", radio(app.group.is_none()))];
    group_rows.extend(
        app.all_groups()
            .into_iter()
            .map(|name| format!("{} {name}", radio(app.group.as_ref() == Some(&name)))),
    );

    filter_list(
        frame,
        repos,
        "repositories",
        repo_rows,
        section == Section::Repos,
        cursor,
    );
    filter_list(
        frame,
        tags,
        "tags (any of)",
        tag_rows,
        section == Section::Tags,
        cursor,
    );
    filter_list(
        frame,
        groups,
        "epic / sprint",
        group_rows,
        section == Section::Groups,
        cursor,
    );
}

fn filter_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<String>,
    active: bool,
    cursor: usize,
) {
    let border = if active {
        Style::new().fg(ACCENT)
    } else {
        Style::new().add_modifier(Modifier::DIM)
    };
    let empty = rows.is_empty();
    let list = List::new(rows)
        .block(
            Block::new()
                .borders(Borders::ALL)
                .border_style(border)
                .title(format!(" {title} ")),
        )
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected((active && !empty).then_some(cursor));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The new/edit form: one row per field, then the body filling the rest.
fn form_screen(frame: &mut Frame, app: &App, form: &Form, area: Rect) {
    let title = match &form.target {
        Target::New { repo } => format!(" new ticket in {} ", app.repos[*repo].name),
        Target::Edit { repo, stem, .. } => {
            format!(" edit {stem} in {} ", app.repos[*repo].name)
        }
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [fields_area, body_area] = Layout::vertical([
        Constraint::Length(form.fields.len() as u16 + 1),
        Constraint::Fill(1),
    ])
    .areas(inner);

    let label_width = form
        .fields
        .iter()
        .map(|field| field.label.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut lines = Vec::new();
    for (at, field) in form.fields.iter().enumerate() {
        let focused = form.focus == at;
        let label = Span::styled(
            format!("{:<label_width$}", format!("{}:", field.label)),
            if focused {
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::DIM)
            },
        );
        let value = match &field.input {
            Input::Text(line) => Span::raw(line.text.clone()),
            Input::Pick(choice) => Span::styled(
                format!("‹ {} ›", choice.value()),
                Style::new().add_modifier(if focused {
                    Modifier::REVERSED
                } else {
                    Modifier::empty()
                }),
            ),
        };
        let mut spans = vec![label, value];
        if focused && !field.hint.is_empty() {
            spans.push(Span::styled(
                format!("   {}", field.hint),
                Style::new().add_modifier(Modifier::DIM),
            ));
        }
        lines.push(Line::from(spans));
        if focused && let Input::Text(line) = &field.input {
            frame.set_cursor_position((
                fields_area.x + label_width as u16 + line.cursor as u16,
                fields_area.y + at as u16,
            ));
        }
    }
    frame.render_widget(Paragraph::new(lines), fields_area);

    let body_block = Block::new()
        .borders(Borders::TOP)
        .border_style(if form.in_body() {
            Style::new().fg(ACCENT)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        })
        .title(" body ");
    let text_area = body_block.inner(body_area);
    frame.render_widget(body_block, body_area);
    // Scroll just enough to keep the cursor's line on screen.
    let height = text_area.height.max(1) as usize;
    let top = form.row.saturating_sub(height - 1);
    let shown: Vec<Line> = form
        .body
        .iter()
        .skip(top)
        .take(height)
        .map(|line| Line::raw(line.text.as_str()))
        .collect();
    frame.render_widget(Paragraph::new(shown), text_area);
    if form.in_body() {
        frame.set_cursor_position((
            text_area.x + form.body[form.row].cursor as u16,
            text_area.y + (form.row - top) as u16,
        ));
    }
}

fn check(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

fn radio(on: bool) -> &'static str {
    if on { "(•)" } else { "( )" }
}

fn help(frame: &mut Frame, area: Rect) {
    let rows = [
        "h / l, ← / →    pick a column",
        "j / k, ↓ / ↑    pick a ticket",
        "g / G           first / last ticket",
        "H / L, < / >    move the ticket one column left / right",
        "Enter           open the ticket (read-only)",
        "n / e           new ticket / edit the selected one",
        "f               filters: repositories, tags, epic / sprint",
        "r               reload from disk",
        "q, Esc          quit",
        "",
        "In the form: Tab / Shift-Tab change field, Ctrl-S saves, Esc cancels,",
        "Ctrl-T ticks or unticks a checklist line, Enter continues a list.",
        "",
        "Moving a ticket does exactly what amd todo / doing / done do:",
        "WIP limit, lease, branch on entering todo, auto-commit.",
    ];
    let width = 66.min(area.width);
    let height = (rows.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let text: Vec<Line> = rows.iter().map(|row| Line::raw(*row)).collect();
    frame.render_widget(
        Paragraph::new(text).block(
            Block::new()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(ACCENT))
                .title(" keys "),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use agile_md::board::Column;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::fixture::repo;

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app(tickets: &[(&str, Column, &str)]) -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path(), tickets);
        (dir, App::new(vec![repo], vec!["elsewhere".to_string()]))
    }

    #[test]
    fn the_board_shows_every_column_with_its_count_and_tickets() {
        let (_dir, app) = app(&[
            ("001-a.md", Column::Todo, ""),
            ("002-b.md", Column::Todo, ""),
            ("003-c.md", Column::Doing, ""),
        ]);
        let screen = render(&app, 120, 12);
        for want in [
            "BACKLOG (0)",
            "TODO (2)",
            "DOING (1)",
            "DONE (0)",
            "001 T001",
            "003 T003",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
        assert!(screen.contains("not cloned: elsewhere"), "{screen}");
    }

    #[test]
    fn an_active_filter_is_named_in_the_header() {
        let (_dir, mut app) = app(&[("001-a.md", Column::Todo, "ux")]);
        app.tags.insert("ux".to_string());
        let screen = render(&app, 120, 8);
        assert!(screen.contains("filter: tags ux"), "{screen}");
    }

    #[test]
    fn the_filter_screen_lists_repositories_tags_and_groups() {
        let (_dir, mut app) = app(&[("001-a.md", Column::Todo, "ux")]);
        app.mode = Mode::Filter {
            section: Section::Tags,
            cursor: 0,
        };
        let screen = render(&app, 120, 10);
        for want in [
            "repositories",
            "[x]",
            "tags (any of)",
            "[ ] ux",
            "(no group filter)",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
    }

    #[test]
    fn a_long_column_scrolls_and_remembers_where() {
        let names: Vec<String> = (1..=30).map(|n| format!("{n:03}-t.md")).collect();
        let tickets: Vec<(&str, Column, &str)> = names
            .iter()
            .map(|n| (n.as_str(), Column::Todo, ""))
            .collect();
        let (_dir, mut app) = app(&tickets);
        app.row = 29;
        let screen = render(&app, 120, 12);
        assert!(app.offsets[1].get() > 0, "the list never scrolled");
        assert!(screen.contains("030 T030"), "{screen}");
        assert!(!screen.contains("001 T001"), "{screen}");
    }

    #[test]
    fn help_is_drawn_over_the_board() {
        let (_dir, mut app) = app(&[]);
        app.mode = Mode::Help;
        let screen = render(&app, 100, 20);
        assert!(screen.contains("keys"), "{screen}");
        assert!(screen.contains("WIP limit, lease"), "{screen}");
    }

    #[test]
    fn the_form_shows_its_fields_and_the_body() {
        let (_dir, mut app) = app(&[("001-a.md", Column::Todo, "ux")]);
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('e'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let screen = render(&app, 100, 24);
        for want in [
            "edit 001-a",
            "Title:",
            "T001",
            "Assignee:",
            "Points:",
            "Branch type:",
            "Epic / sprint:",
            "body of 001",
            "Ctrl-S save",
        ] {
            assert!(screen.contains(want), "missing {want:?} in:\n{screen}");
        }
    }

    #[test]
    fn a_failed_save_is_shown_in_the_footer() {
        let (_dir, mut app) = app(&[]);
        for event in [
            ('n', ratatui::crossterm::event::KeyModifiers::NONE),
            ('s', ratatui::crossterm::event::KeyModifiers::CONTROL),
        ] {
            app.handle_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char(event.0),
                event.1,
            ));
        }
        let screen = render(&app, 100, 24);
        assert!(screen.contains("new ticket in"), "{screen}");
        let last = screen.lines().last().unwrap_or_default().trim().to_string();
        assert!(!last.is_empty() && !last.contains("Ctrl-S"), "{last:?}");
    }
}
