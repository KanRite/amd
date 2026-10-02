//! Rich-text editing of a ticket body, line by line.
//!
//! The markdown stays the document. Every edit is a rewrite of one line of the
//! source, so what is on disk is exactly what you typed — no parse-and-
//! regenerate step that reflows a file the moment you look at it. That matters
//! more here than a richer editor would: the tickets are meant to be read and
//! edited outside this tool as well.
//!
//! One line is being edited at a time; everything else is drawn. Clicking a
//! line starts editing it, Enter commits, and Enter on a list item opens the
//! next one with the same marker.
//!
//! This is the *only* way to edit a ticket body in `amdui` — there is no raw
//! markdown fallback (ticket 069). That makes covering the constructs this
//! project's own tickets actually use non-negotiable, not a nice-to-have:
//! fenced code blocks and `[[wikilinks]]` are already all over this board.
//! Two constructs get genuinely different treatment because they span more
//! than one line, which a per-line classifier can't see on its own:
//! - A fenced code block's content lines are tracked as a running "am I
//!   inside a fence" flag across [`ui`]'s own loop, not by [`classify`] —
//!   `classify` stays a pure, context-free, one-line function, and a stray
//!   `- [ ]`-shaped line *inside* a fence must render as code, not as a
//!   checkbox, which only the loop's own running state can know.
//! - A markdown table's rows classify individually (`Kind::Table`) and are
//!   drawn individually, each in its own bordered row — not accumulated into
//!   one aligned grid across rows. A real grid would need the same kind of
//!   cross-line grouping fences need; a bordered row per line is enough to
//!   stop a table from reading as an unstructured wall of `Kind::Text`,
//!   which is the actual bar ticket 069 sets.

use crate::richtext::{Kind, Span, classify, continuation, join, lines, spans, toggle};

/// Draw a body as rich text, editing one line at a time.
///
/// Returns whether the body changed. `editing` is the line being typed into
/// and whether it has just opened, kept by the caller so it survives between
/// frames — a box that has only just appeared has to ask for focus, or typing
/// would go nowhere.
#[cfg(feature = "gui")]
pub type Editing = Option<(usize, bool)>;

/// The click target for a line with nothing in it. An empty `Label`'s own
/// content is a point, not a rectangle — rendered that way, a blank body
/// line (or an emptied-out list item) is *technically* clickable but has no
/// real hit-box to find, which is exactly what made this editor's blank
/// space unusable once there was no raw-markdown fallback left to fall back
/// to (ticket 069's own follow-up: the parsing was right, this interaction
/// never was). This claims a real, full-width strip instead, with a hint so
/// it doesn't read as dead space either.
///
/// Painting the hint with `ui.painter()` directly (rather than a real
/// `Label` widget) was the first attempt, and it was wrong in a way manual
/// testing alone didn't catch: the painter draws pixels, nothing more — it
/// registers no accessibility node, so the strip was invisible to a screen
/// reader and, not coincidentally, to `egui_kittest`'s own queries (which
/// walk the same AccessKit tree). `response.widget_info` is what a custom
/// widget is for: it attaches the label by hand to the `Response` this
/// function already builds, the same information a `Label` would have
/// registered on its own.
#[cfg(feature = "gui")]
fn empty_line_ui(ui: &mut egui::Ui, enter_edit: &mut bool) {
    const HINT: &str = "click to write…";
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.0), height),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, HINT));
    if ui.is_rect_visible(rect) {
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            HINT,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
    }
    if response.clicked() {
        *enter_edit = true;
    }
}

/// Draw one line's inline spans, each a separately clickable run so a real
/// `[text](url)` link can open the URL instead of starting an edit. `style`
/// applies whatever the *line* kind wants on top (heading size, a struck-
/// through/weak look for a done checklist item, …) to every span alike, so
/// e.g. a **bold** word in a heading still comes out heading-sized.
#[cfg(feature = "gui")]
fn line_ui(
    ui: &mut egui::Ui,
    text: &str,
    style: impl Fn(egui::RichText) -> egui::RichText,
    enter_edit: &mut bool,
) {
    if text.is_empty() {
        return empty_line_ui(ui, enter_edit);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for span in spans(text) {
            let (rich, clickable) = match span {
                Span::Plain(s) => (style(egui::RichText::new(s)), true),
                Span::Bold(s) => (style(egui::RichText::new(s)).strong(), true),
                Span::Italic(s) => (style(egui::RichText::new(s)).italics(), true),
                Span::Code(s) => (style(egui::RichText::new(s)).monospace(), true),
                // Nothing in amdui resolves a wikilink to another ticket yet
                // (nothing did before this ticket either) — styled distinctly
                // so it doesn't read as literal bracket punctuation, same
                // "click anywhere on the line to edit" as everything else on
                // it, rather than a dead end pretending to be a real link.
                Span::WikiLink(target) => (
                    style(egui::RichText::new(format!("[[{target}]]")))
                        .color(ui.visuals().hyperlink_color),
                    true,
                ),
                Span::Link { text, url } => {
                    ui.hyperlink_to(style(egui::RichText::new(text)), url);
                    continue;
                }
            };
            if ui
                .add(egui::Label::new(rich).sense(egui::Sense::click()))
                .clicked()
                && clickable
            {
                *enter_edit = true;
            }
        }
    });
}

#[cfg(feature = "gui")]
pub fn ui(ui: &mut egui::Ui, body: &mut String, editing: &mut Editing) -> bool {
    let mut rows = lines(body);
    let mut changed = false;
    // Whether the *previous* line opened (and didn't yet close) a fenced
    // code block — tracked here, not in `classify`, because a fence spans
    // several lines and `classify` only ever sees one. A line found to be a
    // delimiter itself toggles this but is never treated as fenced content.
    let mut in_fence = false;

    for index in 0..rows.len() {
        let is_fence_delim = matches!(classify(&rows[index]), Kind::Fence { .. });
        let fenced_content = in_fence && !is_fence_delim;
        if is_fence_delim {
            in_fence = !in_fence;
        }

        if editing.map(|(line, _)| line) == Some(index) {
            let id = egui::Id::new(("richtext-line", index));
            let response = ui.add(
                egui::TextEdit::singleline(&mut rows[index])
                    .id(id)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                changed = true;
            }
            // A box that has just appeared has no focus yet; ask once, then
            // stop, or it would steal focus back on every frame.
            if editing.is_some_and(|(_, fresh)| fresh) {
                response.request_focus();
                *editing = Some((index, false));
            }
            let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if entered {
                match continuation(&rows[index]) {
                    Some(prefix) => {
                        rows.insert(index + 1, prefix);
                        *editing = Some((index + 1, true));
                        changed = true;
                    }
                    None => *editing = None,
                }
                break;
            }
            if response.lost_focus() {
                *editing = None;
            }
            continue;
        }

        let line = rows[index].clone();
        let mut enter_edit = false;

        if fenced_content {
            // Deliberately not run through `spans()`/`line_ui`: code is not
            // prose, and parsing `*ptr` or `**kwargs` as markdown formatting
            // would be a real, silent misrendering of the actual content.
            if line.is_empty() {
                empty_line_ui(ui, &mut enter_edit);
            } else {
                let response = ui.add(
                    egui::Label::new(egui::RichText::new(&line).monospace())
                        .sense(egui::Sense::click()),
                );
                if response.clicked() {
                    enter_edit = true;
                }
            }
        } else {
            match classify(&line) {
                Kind::Blank => {
                    empty_line_ui(ui, &mut enter_edit);
                }
                Kind::Fence { lang, .. } => {
                    let label = if lang.is_empty() {
                        "```".to_string()
                    } else {
                        format!("```{lang}")
                    };
                    let response = ui.add(
                        egui::Label::new(egui::RichText::new(label).monospace().weak())
                            .sense(egui::Sense::click()),
                    );
                    if response.clicked() {
                        enter_edit = true;
                    }
                }
                Kind::Heading { level, text } => {
                    let size = match level {
                        1 => 20.0,
                        2 => 17.0,
                        _ => 15.0,
                    };
                    line_ui(
                        ui,
                        &text,
                        move |rich| rich.size(size).strong(),
                        &mut enter_edit,
                    );
                }
                Kind::Check { done, text, indent } => {
                    ui.horizontal(|ui| {
                        ui.add_space(indent.len() as f32 * 8.0);
                        let mut ticked = done;
                        // The box is the one thing you can change without opening
                        // the line for editing — it is the common case by far.
                        if ui.checkbox(&mut ticked, "").changed() {
                            rows[index] = toggle(&line);
                            changed = true;
                        }
                        line_ui(
                            ui,
                            &text,
                            move |rich| {
                                if done {
                                    rich.strikethrough().weak()
                                } else {
                                    rich
                                }
                            },
                            &mut enter_edit,
                        );
                    });
                }
                Kind::Bullet { text, indent, .. } => {
                    ui.horizontal(|ui| {
                        ui.add_space(indent.len() as f32 * 8.0);
                        ui.weak("•");
                        line_ui(ui, &text, |rich| rich, &mut enter_edit);
                    });
                }
                Kind::Numbered {
                    text,
                    indent,
                    number,
                } => {
                    ui.horizontal(|ui| {
                        ui.add_space(indent.len() as f32 * 8.0);
                        ui.weak(format!("{number}."));
                        line_ui(ui, &text, |rich| rich, &mut enter_edit);
                    });
                }
                Kind::Table { cells } => {
                    ui.horizontal(|ui| {
                        for cell in cells {
                            egui::Frame::group(ui.style())
                                .inner_margin(4.0)
                                .show(ui, |ui| line_ui(ui, &cell, |rich| rich, &mut enter_edit));
                        }
                    });
                }
                Kind::Text => {
                    line_ui(ui, &line, |rich| rich, &mut enter_edit);
                }
            }
        }

        if enter_edit {
            *editing = Some((index, true));
        }
    }

    // A body with nowhere to click needs a way in.
    if rows.iter().all(|line| line.trim().is_empty()) && ui.button("Write something…").clicked() {
        *editing = Some((rows.len().saturating_sub(1), true));
    }

    if changed {
        *body = join(&rows);
    }
    changed
}
/// Interaction tests for `ui()` itself — the click-to-edit behaviour the
/// tests above can't reach, since they only exercise `classify`/`spans`.
/// `egui_kittest` runs the real widget tree headless and drives it through
/// its accessibility tree, which is what actually caught ticket 071 (a
/// blank line's click target being too small to find in practice): no
/// display, no compositor, no `xdotool` — a real regression test instead of
/// a screenshot that can only be looked at once, by hand, if the GNOME
/// workflow happens to be cooperating that day.
#[cfg(all(test, feature = "gui"))]
mod interaction_tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// What the harness reads and writes back — the same two pieces of
    /// state `Draft`/`Editor` hand `ui()` in the real app.
    struct State {
        body: String,
        editing: Editing,
    }

    fn harness_with(body: &str) -> Harness<'static, State> {
        let state = State {
            body: body.to_string(),
            editing: None,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(500.0, 400.0))
            .build_ui_state(
                |the_ui, state: &mut State| {
                    ui(the_ui, &mut state.body, &mut state.editing);
                },
                state,
            );
        harness.run();
        harness
    }

    /// The exact shape reported live: the New-ticket template's blank line
    /// under a heading did nothing when clicked, though the heading itself
    /// (clicking "Notes") opened fine for editing. Both are checked here so
    /// a fix for one can't silently break the other. Deliberately a single
    /// blank line, not the full multi-blank-line template — several
    /// identical "click to write…" targets would make the query ambiguous,
    /// which is a test-data problem, not the thing being tested.
    #[test]
    fn clicking_the_blank_line_under_a_heading_opens_it_for_typing() {
        let mut harness = harness_with("## Notes\n");

        harness.get_by_label_contains("click to write").click();
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("Talked to the team".into()));
        harness.run();

        assert!(
            harness.state().body.contains("Talked to the team"),
            "typing after clicking the blank line should reach the body: {:?}",
            harness.state().body
        );
    }

    /// Reported right after the click-target fix landed: Acceptance
    /// criteria could run to several lines (Enter continues a checklist),
    /// but Notes could only ever hold one — Enter just closed the line
    /// instead of opening another, since plain prose had no continuation
    /// at all. `continuation`'s own unit tests cover the string logic;
    /// this proves it end to end, through the actual keypress.
    #[test]
    fn enter_after_a_line_of_notes_opens_a_new_line_to_keep_typing() {
        let mut harness = harness_with("## Notes\n");

        harness.get_by_label_contains("click to write").click();
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("First line".into()));
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("Second line".into()));
        harness.run();

        assert!(
            harness.state().body.contains("First line\nSecond line"),
            "{:?}",
            harness.state().body
        );
    }

    #[test]
    fn clicking_a_heading_edits_the_heading_not_the_line_below() {
        let mut harness = harness_with("\n## Notes\n\n\n## Acceptance criteria\n\n- [ ] \n");

        harness.get_by_label("Notes").click();
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text(" and questions".into()));
        harness.run();

        let body = &harness.state().body;
        assert!(
            body.contains("## Notes and questions"),
            "typing after clicking the heading should edit the heading line: {body:?}"
        );
    }

    /// A list item with nothing typed after its marker yet (`"- [ ] "`, the
    /// template's own starting checklist item) has the same near-empty
    /// click-target problem a blank line does.
    #[test]
    fn clicking_an_empty_checklist_item_opens_it_for_typing() {
        // No trailing newline: one would add a second, ambiguous blank line.
        let mut harness = harness_with("- [ ] ");

        harness.get_by_label_contains("click to write").click();
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("write the ticket".into()));
        harness.run();

        assert!(
            harness.state().body.contains("- [ ] write the ticket"),
            "{:?}",
            harness.state().body
        );
    }

    #[test]
    fn an_empty_line_inside_a_fence_is_still_clickable_and_stays_raw() {
        // No trailing newline after the closing fence: one would add a
        // second, ambiguous blank line outside it.
        let mut harness = harness_with("```rust\n\nlet x = 1;\n```");

        harness.get_by_label_contains("click to write").click();
        harness.run();
        // A literal asterisk, typed into a code line, must never come back
        // out through the inline markdown parser as emphasis — it isn't
        // reachable from here (this only asserts the raw body), but the
        // fence-content branch bypassing `spans()` is what `richtext.rs`'s
        // own module doc promises, and this is the line that would silently
        // stop being true if that branch regressed to going through `line_ui`.
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("*ptr".into()));
        harness.run();

        assert!(
            harness.state().body.contains("*ptr"),
            "{:?}",
            harness.state().body
        );
    }
}
