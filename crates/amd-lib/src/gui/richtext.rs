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
/// stop a list. Only consulted for a single atomic line (a list item, a
/// heading, …) — plain prose doesn't go through here at all any more: a
/// paragraph gets a real multi-line box (see [`ui`]'s own `Paragraph`
/// session), where Enter is a literal line break the widget already
/// handles, not something this function needs an opinion on. (Ticket 074
/// briefly gave `Kind::Text` an arm here — "continue with a blank line" —
/// as a stopgap before 075 built the real thing; superseded, not kept
/// as a second, redundant path.)
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

/// What's currently open for editing, kept by the caller across frames the
/// same way any `TextEdit`'s backing buffer always must be — a box that's
/// only just appeared has to ask for focus once, or typing would go nowhere.
///
/// Two shapes, not one, because they behave genuinely differently rather
/// than just looking different: a `Line` is edited in place (`rows[row]`
/// stays the backing buffer, exactly as it always has); a `Paragraph` needs
/// its own scratch buffer instead; see the variant's own doc for why.
#[cfg(feature = "gui")]
pub enum Session {
    /// A single atomic line — a heading, a checklist/bullet/numbered item,
    /// a fence delimiter, a table cell. Enter asks [`continuation`] what,
    /// if anything, comes next; anything else about losing focus just ends
    /// the session.
    Line { row: usize, fresh: bool },
    /// A contiguous run of prose rows (`start..start + original_len` in the
    /// body as it was when this session opened), edited together as one
    /// auto-expanding multi-line box. `text` is a scratch buffer, not a
    /// borrow of `rows[start]`, *because* its own line count can drift from
    /// `original_len` while it's being typed into — a one-line paragraph
    /// can grow to three, a three-line one can shrink back to one — so
    /// there's no single `rows` slot for a `TextEdit` to borrow directly.
    /// Splitting `text` on `'\n'` and splicing it over the original range
    /// is what commits the result back, on losing focus.
    Paragraph {
        start: usize,
        original_len: usize,
        text: String,
        fresh: bool,
    },
}

#[cfg(feature = "gui")]
pub type Editing = Option<Session>;

/// For every row, whether it's a fenced code block's *content* — never its
/// own delimiter line, which toggles this but is never "inside" itself.
/// Precomputed once per frame rather than tracked inline in [`ui`]'s own
/// loop, so [`paragraph_range`] can consult the same answer without
/// re-deriving it (and, unlike a running boolean, it can be indexed at any
/// row, in any order, which grouping a paragraph range needs to do).
fn fence_mask(rows: &[String]) -> Vec<bool> {
    let mut mask = Vec::with_capacity(rows.len());
    let mut in_fence = false;
    for row in rows {
        let is_delim = matches!(classify(row), Kind::Fence { .. });
        mask.push(in_fence && !is_delim);
        if is_delim {
            in_fence = !in_fence;
        }
    }
    mask
}

/// The contiguous run of `Blank`/`Text` rows that `index` belongs to — a
/// "paragraph," in the sense that matters here: no marker, no fence, no
/// table shape of its own, so it reads (and edits) as one flowing block
/// rather than one clickable strip per source line. A row inside a fenced
/// code block is never included even if it would otherwise classify as
/// `Blank`/`Text` in isolation — `fenced` (from [`fence_mask`]) is what
/// keeps code out of prose grouping, the same as it already keeps code out
/// of the inline markdown parser.
fn paragraph_range(rows: &[String], fenced: &[bool], index: usize) -> std::ops::Range<usize> {
    let is_prose = |i: usize| !fenced[i] && matches!(classify(&rows[i]), Kind::Blank | Kind::Text);
    let mut start = index;
    while start > 0 && is_prose(start - 1) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < rows.len() && is_prose(end) {
        end += 1;
    }
    start..end
}

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
    let fenced = fence_mask(&rows);

    let mut index = 0;
    while index < rows.len() {
        match editing {
            Some(Session::Line { row, fresh }) if *row == index => {
                let id = egui::Id::new(("richtext-line", index));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut rows[index])
                        .id(id)
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    changed = true;
                }
                // A box that has just appeared has no focus yet; ask once,
                // then stop, or it would steal focus back on every frame.
                if *fresh {
                    response.request_focus();
                    *fresh = false;
                }
                let entered =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if entered {
                    match continuation(&rows[index]) {
                        Some(prefix) => {
                            rows.insert(index + 1, prefix);
                            *editing = Some(Session::Line {
                                row: index + 1,
                                fresh: true,
                            });
                            changed = true;
                        }
                        None => *editing = None,
                    }
                    break;
                }
                if response.lost_focus() {
                    *editing = None;
                }
                index += 1;
                continue;
            }
            Some(Session::Paragraph {
                start,
                original_len,
                text,
                fresh,
            }) if *start == index => {
                let id = egui::Id::new(("richtext-paragraph", index));
                let response = ui.add(
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .desired_width(f32::INFINITY)
                        .desired_rows(1),
                );
                if response.changed() {
                    changed = true;
                }
                if *fresh {
                    response.request_focus();
                    *fresh = false;
                }
                // Enter is a literal line break inside a multiline box —
                // `TextEdit`'s own default, nothing to intercept here.
                // Losing focus (clicking away, Tab, Escape) is what commits:
                // split the scratch buffer back into rows and splice it
                // over the range this session opened with. The split can
                // land on a different number of rows than it started with —
                // that's the entire point of a scratch buffer instead of
                // borrowing `rows[start]` directly.
                if response.lost_focus() {
                    let new_rows: Vec<String> = text.split('\n').map(str::to_string).collect();
                    rows.splice(*start..*start + *original_len, new_rows);
                    *editing = None;
                    changed = true;
                    break;
                }
                // The whole original range is one widget while it's open;
                // nothing left inside it to draw separately this frame.
                index = *start + *original_len;
                continue;
            }
            _ => {}
        }

        let fenced_content = fenced[index];

        if fenced_content {
            // Deliberately not run through `spans()`/`line_ui`/paragraph
            // grouping: code is not prose, and parsing `*ptr` or `**kwargs`
            // as markdown formatting would be a real, silent misrendering
            // of the actual content.
            let line = rows[index].clone();
            let mut enter_edit = false;
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
            if enter_edit {
                *editing = Some(Session::Line {
                    row: index,
                    fresh: true,
                });
            }
            index += 1;
            continue;
        }

        if matches!(classify(&rows[index]), Kind::Blank | Kind::Text) {
            // A run of plain prose reads — and edits — as one block, not
            // one clickable strip per source line (ticket 075: the
            // per-line model suits headings/lists/tables/fences, which are
            // each genuinely one unit, but not a paragraph, which isn't).
            let range = paragraph_range(&rows, &fenced, index);
            let mut enter_edit = false;
            for i in range.clone() {
                let line = rows[i].clone();
                line_ui(ui, &line, |rich| rich, &mut enter_edit);
            }
            if enter_edit {
                *editing = Some(Session::Paragraph {
                    start: range.start,
                    original_len: range.len(),
                    text: join(&rows[range.clone()]),
                    fresh: true,
                });
            }
            index = range.end;
            continue;
        }

        let line = rows[index].clone();
        let mut enter_edit = false;

        match classify(&line) {
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
            // Handled above, before this match — reaching either here would
            // be a real bug, not a case to render some other way.
            Kind::Blank | Kind::Text => {
                unreachable!("paragraph_range's Blank|Text branch already continue'd past this row")
            }
        }

        if enter_edit {
            *editing = Some(Session::Line {
                row: index,
                fresh: true,
            });
        }
        index += 1;
    }

    // A body with nowhere to click needs a way in. With nothing else on the
    // board, the whole thing is one (empty) paragraph — a `Line` session
    // wouldn't even have a single-line box to reuse the way a normal atomic
    // edit does, since there is no atomic line here at all.
    if rows.iter().all(|line| line.trim().is_empty()) && ui.button("Write something…").clicked() {
        *editing = Some(Session::Paragraph {
            start: 0,
            original_len: rows.len(),
            text: join(&rows),
            fresh: true,
        });
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
        // A heading does not become a list — or anything else; see
        // `headings_do_not_continue` below for why that stays `None` while
        // plain text now continues with a blank line.
        assert_eq!(continuation("## Notes"), None);
        // An empty item ends the list rather than making another.
        assert_eq!(continuation("- [ ] "), None);
        assert_eq!(continuation("- "), None);
    }

    /// Plain prose isn't continued via `continuation` at all any more
    /// (ticket 075 superseded 074's stopgap with a real multi-line box) —
    /// confirms the arm is actually gone, not just unreachable in practice.
    #[test]
    fn continuation_has_no_opinion_on_plain_text() {
        assert_eq!(continuation("just words"), None);
        assert_eq!(continuation("  indented words"), None);
    }

    #[test]
    fn paragraph_range_groups_contiguous_prose_and_stops_at_structure() {
        let rows: Vec<String> = lines("## Notes\n\nfirst\nsecond\n\n## AC\n- [ ] x\n\nmore");
        let fenced = fence_mask(&rows);
        // rows: ["## Notes","","first","second","","## AC","- [ ] x","","more"]
        // The blank line and both prose lines between the two headings are
        // one contiguous run, regardless of which of the three is clicked.
        assert_eq!(paragraph_range(&rows, &fenced, 1), 1..5);
        assert_eq!(paragraph_range(&rows, &fenced, 2), 1..5);
        assert_eq!(paragraph_range(&rows, &fenced, 3), 1..5);
        assert_eq!(paragraph_range(&rows, &fenced, 4), 1..5);
        // A checklist item breaks the run — a separate paragraph starts
        // fresh after it, not merged across the list item.
        assert_eq!(paragraph_range(&rows, &fenced, 7), 7..9);
    }

    #[test]
    fn paragraph_range_excludes_fenced_content() {
        let rows: Vec<String> = lines("prose\n```\nnot prose\n```\nmore prose");
        let fenced = fence_mask(&rows);
        assert_eq!(fenced, [false, false, true, false, false]);
        assert_eq!(paragraph_range(&rows, &fenced, 0), 0..1);
        assert_eq!(paragraph_range(&rows, &fenced, 4), 4..5);
    }

    #[test]
    fn headings_do_not_continue() {
        assert_eq!(continuation("## Notes"), None);
        assert_eq!(continuation("# Title"), None);
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
        // A paragraph session (ticket 075) commits on losing focus, the
        // same as any other text box — Escape is the standard way to blur
        // one deliberately rather than by clicking some other widget.
        harness.key_press(egui::Key::Escape);
        harness.run();

        assert!(
            harness.state().body.contains("Talked to the team"),
            "typing after clicking the blank line should reach the body: {:?}",
            harness.state().body
        );
    }

    /// Ticket 075: Notes gets a real, auto-expanding multi-line box — Enter
    /// is a literal line break `TextEdit::multiline` already handles, not
    /// something this editor has to intercept (contrast ticket 074's
    /// earlier stopgap, superseded, which asked `continuation` to open a
    /// new single-line row per keypress instead).
    #[test]
    fn enter_inside_a_paragraph_is_a_real_line_break() {
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
        harness.key_press(egui::Key::Escape);
        harness.run();

        assert!(
            harness.state().body.contains("First line\nSecond line"),
            "{:?}",
            harness.state().body
        );
    }

    /// The scratch buffer growing from one row to several has to *splice*
    /// over the original range, not just overwrite `rows[start]` — a
    /// one-line paragraph becoming three is two extra rows that everything
    /// after it (here, the Acceptance criteria heading and its checklist)
    /// needs to shift past, not get silently overwritten by.
    #[test]
    fn a_growing_paragraph_does_not_corrupt_what_comes_after_it() {
        let mut harness = harness_with("## Notes\n\n## Acceptance criteria\n- [ ] first");

        harness.get_by_label_contains("click to write").click();
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("one".into()));
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("two".into()));
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("three".into()));
        harness.run();
        harness.key_press(egui::Key::Escape);
        harness.run();

        let body = &harness.state().body;
        assert!(body.contains("one\ntwo\nthree"), "{body:?}");
        assert!(
            body.contains("## Acceptance criteria\n- [ ] first"),
            "the checklist below must survive untouched: {body:?}"
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
