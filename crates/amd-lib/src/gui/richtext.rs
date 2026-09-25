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

/// What a line of markdown is, as far as the editor cares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Heading {
        level: usize,
        text: String,
    },
    /// A `- [ ]` / `- [x]` item, with its indent preserved.
    Check {
        indent: String,
        done: bool,
        text: String,
    },
    Bullet {
        indent: String,
        marker: String,
        text: String,
    },
    Blank,
    Text,
}

/// Classify one line. Deliberately forgiving: anything unrecognised is plain
/// text, which is drawn as-is and edits as-is.
pub fn classify(line: &str) -> Kind {
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let rest = &line[indent.len()..];

    if rest.trim().is_empty() {
        return Kind::Blank;
    }
    if let Some(hashes) = rest.strip_prefix('#') {
        let extra = hashes.chars().take_while(|c| *c == '#').count();
        let text = hashes[extra..].trim_start();
        // "#hashtag" is not a heading; a heading has a space after the hashes.
        if hashes[extra..].starts_with(' ') || text.is_empty() {
            return Kind::Heading {
                level: extra + 1,
                text: text.to_string(),
            };
        }
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(after) = rest.strip_prefix(marker) {
            if let Some(box_rest) = after.strip_prefix("[ ]").or(after.strip_prefix("[]")) {
                return Kind::Check {
                    indent,
                    done: false,
                    text: box_rest.trim_start().to_string(),
                };
            }
            if let Some(box_rest) = after.strip_prefix("[x]").or(after.strip_prefix("[X]")) {
                return Kind::Check {
                    indent,
                    done: true,
                    text: box_rest.trim_start().to_string(),
                };
            }
            return Kind::Bullet {
                indent,
                marker: marker.trim_end().to_string(),
                text: after.to_string(),
            };
        }
    }
    Kind::Text
}

/// Flip a checkbox, leaving the rest of the line — indent, marker, text —
/// exactly as it was.
pub fn toggle(line: &str) -> String {
    match classify(line) {
        Kind::Check { indent, done, text } => {
            let box_ = if done { "[ ]" } else { "[x]" };
            format!("{indent}- {box_} {text}")
        }
        _ => line.to_string(),
    }
}

/// What pressing Enter at the end of this line should open next.
///
/// `Some("- [ ] ")` after a checklist item, `Some("- ")` after a bullet, and
/// `None` after anything else — including an empty list item, where continuing
/// would mean fighting the editor to stop a list.
pub fn continuation(line: &str) -> Option<String> {
    match classify(line) {
        Kind::Check { indent, text, .. } if !text.trim().is_empty() => {
            Some(format!("{indent}- [ ] "))
        }
        Kind::Bullet {
            indent,
            marker,
            text,
        } if !text.trim().is_empty() => Some(format!("{indent}{marker} ")),
        _ => None,
    }
}

/// Split a body into lines for editing, keeping the trailing blank so a body
/// that ends in a newline still does after a round trip.
pub fn lines(body: &str) -> Vec<String> {
    body.split('\n').map(str::to_string).collect()
}

pub fn join(lines: &[String]) -> String {
    lines.join("\n")
}

/// Draw a body as rich text, editing one line at a time.
///
/// Returns whether the body changed. `editing` is the line being typed into
/// and whether it has just opened, kept by the caller so it survives between
/// frames — a box that has only just appeared has to ask for focus, or typing
/// would go nowhere.
#[cfg(feature = "gui")]
pub type Editing = Option<(usize, bool)>;

#[cfg(feature = "gui")]
pub fn ui(ui: &mut egui::Ui, body: &mut String, editing: &mut Editing) -> bool {
    let mut rows = lines(body);
    let mut changed = false;

    for index in 0..rows.len() {
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
        match classify(&line) {
            Kind::Blank => {
                // Still clickable, or an empty line could never be typed into.
                let response = ui.add(egui::Label::new(" ").sense(egui::Sense::click()));
                if response.clicked() {
                    *editing = Some((index, true));
                }
            }
            Kind::Heading { level, text } => {
                let size = match level {
                    1 => 20.0,
                    2 => 17.0,
                    _ => 15.0,
                };
                let response = ui.add(
                    egui::Label::new(egui::RichText::new(text).size(size).strong())
                        .sense(egui::Sense::click()),
                );
                if response.clicked() {
                    *editing = Some((index, true));
                }
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
                    let label = if done {
                        egui::RichText::new(text).strikethrough().weak()
                    } else {
                        egui::RichText::new(text)
                    };
                    if ui
                        .add(egui::Label::new(label).sense(egui::Sense::click()))
                        .clicked()
                    {
                        *editing = Some((index, true));
                    }
                });
            }
            Kind::Bullet { text, indent, .. } => {
                ui.horizontal(|ui| {
                    ui.add_space(indent.len() as f32 * 8.0);
                    ui.weak("•");
                    if ui
                        .add(egui::Label::new(text).sense(egui::Sense::click()))
                        .clicked()
                    {
                        *editing = Some((index, true));
                    }
                });
            }
            Kind::Text => {
                let response = ui.add(egui::Label::new(line.clone()).sense(egui::Sense::click()));
                if response.clicked() {
                    *editing = Some((index, true));
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkboxes_are_recognised_either_way_round() {
        assert_eq!(
            classify("- [ ] wash up"),
            Kind::Check {
                indent: String::new(),
                done: false,
                text: "wash up".into()
            }
        );
        assert_eq!(
            classify("  - [x] done it"),
            Kind::Check {
                indent: "  ".into(),
                done: true,
                text: "done it".into()
            }
        );
    }

    #[test]
    fn toggling_preserves_everything_but_the_box() {
        assert_eq!(toggle("- [ ] wash up"), "- [x] wash up");
        assert_eq!(toggle("- [x] wash up"), "- [ ] wash up");
        assert_eq!(toggle("  - [x] indented"), "  - [ ] indented");
        // Not a checkbox: left alone rather than mangled.
        assert_eq!(toggle("- a bullet"), "- a bullet");
        assert_eq!(toggle("## A heading"), "## A heading");
    }

    #[test]
    fn a_body_survives_a_round_trip_untouched() {
        let body = "\n## Notes\n\n- [ ] one\n- [x] two\n\ntrailing words\n";
        assert_eq!(join(&lines(body)), body, "the source is the document");
    }

    #[test]
    fn enter_continues_a_list_but_does_not_start_one() {
        assert_eq!(continuation("- [ ] one").as_deref(), Some("- [ ] "));
        assert_eq!(continuation("- one").as_deref(), Some("- "));
        assert_eq!(continuation("  * one").as_deref(), Some("  * "));
        // Plain text and headings do not become lists.
        assert_eq!(continuation("just words"), None);
        assert_eq!(continuation("## Notes"), None);
        // An empty item ends the list rather than making another.
        assert_eq!(continuation("- [ ] "), None);
        assert_eq!(continuation("- "), None);
    }

    #[test]
    fn headings_need_a_space_so_hashtags_stay_text() {
        assert_eq!(
            classify("### Acceptance criteria"),
            Kind::Heading {
                level: 3,
                text: "Acceptance criteria".into()
            }
        );
        assert_eq!(classify("#hashtag"), Kind::Text);
    }

    #[test]
    fn a_bullet_keeps_its_marker_when_continued() {
        // Continuing a "*" list with "-" would rewrite the author's style.
        assert_eq!(continuation("* one").as_deref(), Some("* "));
        assert_eq!(continuation("+ one").as_deref(), Some("+ "));
    }
}
