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
    /// `1. text` — `number` is kept as written (not parsed) so a line that
    /// isn't being edited round-trips byte for byte; [`continuation`] is
    /// where it actually gets parsed, to work out the next number.
    Numbered {
        indent: String,
        number: String,
        text: String,
    },
    /// A fenced-code delimiter line (opening or closing — the two look
    /// identical in isolation, which is exactly why [`ui`] has to track
    /// which one it is itself). `lang` is empty on a closing fence.
    Fence {
        indent: String,
        lang: String,
    },
    /// One row of a markdown table (a header row, a separator row, or a data
    /// row — `classify` can't tell them apart on its own, and doesn't try).
    Table {
        cells: Vec<String>,
    },
    Blank,
    Text,
}

/// Classify one line. Deliberately forgiving: anything unrecognised is plain
/// text, which is drawn as-is and edits as-is.
///
/// Context-free by design — see this module's own doc comment for why fenced
/// code and tables need extra state that lives in [`ui`] instead.
pub fn classify(line: &str) -> Kind {
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let rest = &line[indent.len()..];

    if rest.trim().is_empty() {
        return Kind::Blank;
    }
    if let Some(lang) = rest.strip_prefix("```") {
        return Kind::Fence {
            indent,
            lang: lang.trim().to_string(),
        };
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
    // A table row, the shape this project's own tickets actually use:
    // starts and ends with `|`. Anything looser (a bare `a | b`, no edge
    // pipes) is ambiguous with ordinary text that happens to contain a pipe,
    // so it's left as Text rather than guessed at.
    let trimmed = rest.trim_end();
    if rest.starts_with('|') && trimmed.len() > 1 && trimmed.ends_with('|') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let cells = inner
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect();
        return Kind::Table { cells };
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty()
        && let Some(text) = rest[digits.len()..].strip_prefix(". ")
    {
        return Kind::Numbered {
            indent,
            number: digits,
            text: text.to_string(),
        };
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
/// `Some("- [ ] ")` after a checklist item, `Some("- ")` after a bullet,
/// `Some("2. ")` after `"1. "`, and `None` after anything else — including
/// an empty list item, where continuing would mean fighting the editor to
/// stop a list.
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
        Kind::Numbered {
            indent,
            number,
            text,
        } if !text.trim().is_empty() => {
            // An unparseable number (nobody types "1." with a number this
            // large) just stops the list rather than panicking or wrapping.
            let next = number.parse::<u64>().ok()?.checked_add(1)?;
            Some(format!("{indent}{next}. "))
        }
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

/// One run of inline formatting within a line, as far as *display* cares —
/// never fed back into the source, which is why this can be as forgiving as
/// it likes: a construct it doesn't recognise just stays plain text, exactly
/// as typed, the same "unrecognised is fine" philosophy [`classify`] uses at
/// the line level.
#[derive(Debug, PartialEq)]
enum Span {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
    WikiLink(String),
}

/// Find the next occurrence of `needle` in `chars` at or after `from`.
fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || from + needle.len() > chars.len() {
        return None;
    }
    (from..=chars.len() - needle.len()).find(|&i| chars[i..i + needle.len()] == needle[..])
}

/// Parse `text` into inline spans: code spans first (so formatting markers
/// inside `` `code` `` are never misread as real formatting), then
/// `[[wikilinks]]`, then `[links](url)`, then `**bold**`, then `*italic*`/
/// `_italic_`. Nothing here nests — a ticket body is prose, not a markdown
/// torture test, and the fallback for anything stranger is always "leave it
/// as plain text", never a panic or a dropped character.
fn spans(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '`'
            && let Some(end) = find(&chars, i + 1, "`")
        {
            if !plain.is_empty() {
                out.push(Span::Plain(std::mem::take(&mut plain)));
            }
            out.push(Span::Code(chars[i + 1..end].iter().collect()));
            i = end + 1;
            continue;
        }
        if chars[i] == '['
            && chars.get(i + 1) == Some(&'[')
            && let Some(end) = find(&chars, i + 2, "]]")
        {
            if !plain.is_empty() {
                out.push(Span::Plain(std::mem::take(&mut plain)));
            }
            out.push(Span::WikiLink(chars[i + 2..end].iter().collect()));
            i = end + 2;
            continue;
        }
        if chars[i] == '['
            && let Some(close) = find(&chars, i + 1, "]")
            && chars.get(close + 1) == Some(&'(')
            && let Some(paren) = find(&chars, close + 2, ")")
        {
            if !plain.is_empty() {
                out.push(Span::Plain(std::mem::take(&mut plain)));
            }
            out.push(Span::Link {
                text: chars[i + 1..close].iter().collect(),
                url: chars[close + 2..paren].iter().collect(),
            });
            i = paren + 1;
            continue;
        }
        if chars[i] == '*'
            && chars.get(i + 1) == Some(&'*')
            && let Some(end) = find(&chars, i + 2, "**")
        {
            if !plain.is_empty() {
                out.push(Span::Plain(std::mem::take(&mut plain)));
            }
            out.push(Span::Bold(chars[i + 2..end].iter().collect()));
            i = end + 2;
            continue;
        }
        if (chars[i] == '*' || chars[i] == '_')
            && let Some(end) = find(&chars, i + 1, &chars[i].to_string())
            && end > i + 1
        {
            if !plain.is_empty() {
                out.push(Span::Plain(std::mem::take(&mut plain)));
            }
            out.push(Span::Italic(chars[i + 1..end].iter().collect()));
            i = end + 1;
            continue;
        }
        plain.push(chars[i]);
        i += 1;
    }
    if !plain.is_empty() {
        out.push(Span::Plain(plain));
    }
    out
}

/// Draw a body as rich text, editing one line at a time.
///
/// Returns whether the body changed. `editing` is the line being typed into
/// and whether it has just opened, kept by the caller so it survives between
/// frames — a box that has only just appeared has to ask for focus, or typing
/// would go nowhere.
#[cfg(feature = "gui")]
pub type Editing = Option<(usize, bool)>;

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
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // An empty line still needs something to click, or a freshly-opened
        // empty list item could never be reopened once it lost focus.
        let text_spans = if text.is_empty() {
            vec![Span::Plain(String::new())]
        } else {
            spans(text)
        };
        for span in text_spans {
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
            // Deliberately not run through `spans()`: code is not prose, and
            // parsing `*ptr` or `**kwargs` as markdown formatting would be a
            // real, silent misrendering of the actual content.
            let shown = if line.is_empty() { " " } else { &line };
            let response = ui.add(
                egui::Label::new(egui::RichText::new(shown).monospace())
                    .sense(egui::Sense::click()),
            );
            if response.clicked() {
                enter_edit = true;
            }
        } else {
            match classify(&line) {
                Kind::Blank => {
                    // Still clickable, or an empty line could never be typed into.
                    let response = ui.add(egui::Label::new(" ").sense(egui::Sense::click()));
                    if response.clicked() {
                        enter_edit = true;
                    }
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

    #[test]
    fn numbered_items_are_recognised_and_continue_incrementing() {
        assert_eq!(
            classify("1. first"),
            Kind::Numbered {
                indent: String::new(),
                number: "1".into(),
                text: "first".into()
            }
        );
        assert_eq!(
            classify("  12. indented"),
            Kind::Numbered {
                indent: "  ".into(),
                number: "12".into(),
                text: "indented".into()
            }
        );
        assert_eq!(continuation("1. first").as_deref(), Some("2. "));
        assert_eq!(continuation("9. ninth").as_deref(), Some("10. "));
        // An empty item ends the list, same as a bullet or checklist would.
        assert_eq!(continuation("1. "), None);
        // A bare number with no ". " after it is not a list marker.
        assert_eq!(classify("1) not this shape"), Kind::Text);
        assert_eq!(classify("2025 not a list either"), Kind::Text);
    }

    #[test]
    fn fenced_code_delimiters_are_recognised_open_and_close() {
        assert_eq!(
            classify("```rust"),
            Kind::Fence {
                indent: String::new(),
                lang: "rust".into()
            }
        );
        assert_eq!(
            classify("```"),
            Kind::Fence {
                indent: String::new(),
                lang: String::new()
            }
        );
        assert_eq!(
            classify("  ```"),
            Kind::Fence {
                indent: "  ".into(),
                lang: String::new()
            }
        );
    }

    #[test]
    fn table_rows_split_into_trimmed_cells() {
        assert_eq!(
            classify("| a | b c | d |"),
            Kind::Table {
                cells: vec!["a".into(), "b c".into(), "d".into()]
            }
        );
        assert_eq!(
            classify("| --- | --- |"),
            Kind::Table {
                cells: vec!["---".into(), "---".into()]
            }
        );
        // No edge pipes: ambiguous with ordinary text, left as Text.
        assert_eq!(classify("a | b"), Kind::Text);
    }

    #[test]
    fn inline_spans_recognise_code_links_wikilinks_and_emphasis() {
        assert_eq!(
            spans("plain `code` **bold** *italic* _also italic_"),
            vec![
                Span::Plain("plain ".into()),
                Span::Code("code".into()),
                Span::Plain(" ".into()),
                Span::Bold("bold".into()),
                Span::Plain(" ".into()),
                Span::Italic("italic".into()),
                Span::Plain(" ".into()),
                Span::Italic("also italic".into()),
            ]
        );
        assert_eq!(
            spans("see [[042-some-ticket]] and [docs](https://example.com)"),
            vec![
                Span::Plain("see ".into()),
                Span::WikiLink("042-some-ticket".into()),
                Span::Plain(" and ".into()),
                Span::Link {
                    text: "docs".into(),
                    url: "https://example.com".into()
                },
            ]
        );
    }

    #[test]
    fn inline_spans_leave_unmatched_markers_as_plain_text() {
        // No closing marker: not a panic, not a dropped character.
        assert_eq!(
            spans("a `stray backtick"),
            vec![Span::Plain("a `stray backtick".into())]
        );
        assert_eq!(
            spans("half [bracket"),
            vec![Span::Plain("half [bracket".into())]
        );
        assert_eq!(
            spans("an * unmatched star"),
            vec![Span::Plain("an * unmatched star".into())]
        );
        // Code spans win over everything inside them.
        assert_eq!(
            spans("`**not bold**`"),
            vec![Span::Code("**not bold**".into())]
        );
    }
}
