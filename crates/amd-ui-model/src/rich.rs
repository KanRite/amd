//! The rich-text editor's document model.
//!
//! A ticket body travels `markdown → Vec<Block> → HTML → (edited in the
//! WebView) → Vec<Block> → markdown`. Owning both conversions here — rather
//! than handing raw HTML to the editor — keeps the HTML to a small, known set
//! of elements the frontend can read back, and keeps every conversion
//! testable without a browser.
//!
//! The model covers what tickets use: headings, paragraphs, bullet, numbered
//! and checklist items, quotes, code, rules, and bold/italic/code/link text.
//! Anything else (tables, raw HTML) is kept as plain text, never dropped.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Code(String),
    Link { href: String, children: Vec<Inline> },
    LineBreak,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Level 1–6.
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Code(String),
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(done)` for a checklist item, `None` for a plain one.
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

// ---------------------------------------------------------------- markdown → model

/// Parses a ticket body (no frontmatter) into blocks.
pub fn parse_markdown(markdown: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    // No strikethrough: `~~` has no element in the model, so it stays literal.
    opts.insert(Options::ENABLE_TASKLISTS);
    let events: Vec<Event<'_>> = Parser::new_ext(markdown, opts).collect();
    let mut pos = 0;
    parse_blocks(&events, &mut pos, None)
}

/// Reads blocks until the event that closes `until` (or the end).
fn parse_blocks(events: &[Event<'_>], pos: &mut usize, until: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();
    // Loose text directly inside a list item (a "tight" list has no
    // paragraph events) is gathered into one paragraph.
    let mut loose: Vec<Inline> = Vec::new();
    while *pos < events.len() {
        let event = &events[*pos];
        *pos += 1;
        match event {
            Event::End(end) if until.is_some_and(|u| closes(end, &u)) => break,
            Event::Start(Tag::Paragraph) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Paragraph(parse_inlines(
                    events,
                    pos,
                    TagEnd::Paragraph,
                )));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut loose, &mut blocks);
                let end = TagEnd::Heading(*level);
                blocks.push(Block::Heading(
                    heading_number(*level),
                    parse_inlines(events, pos, end),
                ));
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut loose, &mut blocks);
                blocks.push(parse_list(events, pos, start.is_some()));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Quote(parse_blocks(
                    events,
                    pos,
                    Some(TagEnd::BlockQuote(None)),
                )));
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut loose, &mut blocks);
                let mut code = String::new();
                while let Some(ev) = events.get(*pos) {
                    *pos += 1;
                    match ev {
                        Event::Text(t) => code.push_str(t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                blocks.push(Block::Code(code.trim_end_matches('\n').to_string()));
            }
            Event::Rule => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Rule);
            }
            Event::Html(html) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Paragraph(vec![Inline::Text(
                    html.trim_end().to_string(),
                )]));
            }
            // Inline formatting directly inside a tight list item.
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Link { .. } | Tag::Image { .. }) => {
                *pos -= 1;
                if let Some(inline) = parse_inline(events, pos) {
                    loose.push(inline);
                }
            }
            // Containers this model has no element for: keep their text.
            Event::Start(_) | Event::End(_) => {}
            _ => {
                *pos -= 1;
                if let Some(inline) = parse_inline(events, pos) {
                    loose.push(inline);
                }
            }
        }
    }
    flush(&mut loose, &mut blocks);
    blocks
}

/// Whether `end` closes the container `until`. A quote's end carries its
/// alert kind, so any quote end closes a quote.
fn closes(end: &TagEnd, until: &TagEnd) -> bool {
    end == until || matches!((end, until), (TagEnd::BlockQuote(_), TagEnd::BlockQuote(_)))
}

fn flush(loose: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    if !loose.is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(loose)));
    }
}

fn parse_list(events: &[Event<'_>], pos: &mut usize, ordered: bool) -> Block {
    let mut items = Vec::new();
    while let Some(event) = events.get(*pos) {
        *pos += 1;
        match event {
            Event::Start(Tag::Item) => {
                let mut checked = None;
                if let Some(Event::TaskListMarker(done)) = events.get(*pos) {
                    checked = Some(*done);
                    *pos += 1;
                }
                let blocks = parse_blocks(events, pos, Some(TagEnd::Item));
                items.push(ListItem { checked, blocks });
            }
            Event::End(TagEnd::List(_)) => break,
            _ => {}
        }
    }
    Block::List { ordered, items }
}

fn parse_inlines(events: &[Event<'_>], pos: &mut usize, end: TagEnd) -> Vec<Inline> {
    let mut out = Vec::new();
    while let Some(event) = events.get(*pos) {
        if matches!(event, Event::End(e) if *e == end) {
            *pos += 1;
            break;
        }
        if let Some(inline) = parse_inline(events, pos) {
            out.push(inline);
        }
    }
    merge_text(out)
}

/// Parses one inline starting at `events[*pos]`, advancing past it.
fn parse_inline(events: &[Event<'_>], pos: &mut usize) -> Option<Inline> {
    let event = events.get(*pos)?;
    *pos += 1;
    Some(match event {
        Event::Text(t) => Inline::Text(t.to_string()),
        Event::Code(c) => Inline::Code(c.to_string()),
        Event::SoftBreak => Inline::Text(" ".into()),
        Event::HardBreak => Inline::LineBreak,
        Event::InlineHtml(h) | Event::Html(h) => Inline::Text(h.to_string()),
        Event::TaskListMarker(done) => Inline::Text(if *done { "[x] " } else { "[ ] " }.into()),
        Event::Start(Tag::Strong) => Inline::Bold(parse_inlines(events, pos, TagEnd::Strong)),
        Event::Start(Tag::Emphasis) => Inline::Italic(parse_inlines(events, pos, TagEnd::Emphasis)),
        Event::Start(Tag::Link { dest_url, .. }) => Inline::Link {
            href: dest_url.to_string(),
            children: parse_inlines(events, pos, TagEnd::Link),
        },
        Event::Start(Tag::Image { dest_url, .. }) => {
            let alt = parse_inlines(events, pos, TagEnd::Image);
            Inline::Text(format!("![{}]({dest_url})", plain_text(&alt)))
        }
        _ => return None,
    })
}

fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (out.last_mut(), inline) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => prev.push_str(&next),
            (_, inline) => out.push(inline),
        }
    }
    out
}

fn plain_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t) | Inline::Code(t) => t.clone(),
            Inline::Bold(c) | Inline::Italic(c) | Inline::Link { children: c, .. } => plain_text(c),
            Inline::LineBreak => " ".into(),
        })
        .collect()
}

fn heading_number(level: HeadingLevel) -> u8 {
    level as u8
}

// ---------------------------------------------------------------- model → HTML

/// Renders blocks as the HTML the rich editor edits. Every piece of text is
/// escaped, so ticket content can never inject markup into the WebView.
pub fn to_html(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        block_html(block, &mut out);
    }
    out
}

fn block_html(block: &Block, out: &mut String) {
    match block {
        Block::Heading(level, inlines) => {
            out.push_str(&format!("<h{level}>"));
            inlines_html(inlines, out);
            out.push_str(&format!("</h{level}>"));
        }
        Block::Paragraph(inlines) => {
            out.push_str("<p>");
            inlines_html(inlines, out);
            if inlines.is_empty() {
                out.push_str("<br>");
            }
            out.push_str("</p>");
        }
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            out.push_str(&format!("<{tag}>"));
            for item in items {
                match item.checked {
                    Some(done) => {
                        out.push_str("<li data-task=\"true\"><input type=\"checkbox\" contenteditable=\"false\"");
                        if done {
                            out.push_str(" checked");
                        }
                        out.push('>');
                    }
                    None => out.push_str("<li>"),
                }
                // A single paragraph renders inline so the item stays one line.
                match item.blocks.as_slice() {
                    [Block::Paragraph(inlines)] => inlines_html(inlines, out),
                    blocks => {
                        for b in blocks {
                            block_html(b, out);
                        }
                    }
                }
                out.push_str("</li>");
            }
            out.push_str(&format!("</{tag}>"));
        }
        Block::Quote(blocks) => {
            out.push_str("<blockquote>");
            for b in blocks {
                block_html(b, out);
            }
            out.push_str("</blockquote>");
        }
        Block::Code(code) => {
            out.push_str("<pre>");
            escape_into(code, out);
            out.push_str("</pre>");
        }
        Block::Rule => out.push_str("<hr>"),
    }
}

fn inlines_html(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(t) => escape_into(t, out),
            Inline::Bold(c) => {
                out.push_str("<strong>");
                inlines_html(c, out);
                out.push_str("</strong>");
            }
            Inline::Italic(c) => {
                out.push_str("<em>");
                inlines_html(c, out);
                out.push_str("</em>");
            }
            Inline::Code(c) => {
                out.push_str("<code>");
                escape_into(c, out);
                out.push_str("</code>");
            }
            Inline::Link { href, children } => {
                out.push_str("<a href=\"");
                escape_into(href, out);
                out.push_str("\">");
                inlines_html(children, out);
                out.push_str("</a>");
            }
            Inline::LineBreak => out.push_str("<br>"),
        }
    }
}

fn escape_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

// ---------------------------------------------------------------- model → markdown

/// Writes blocks back as markdown in the style `amd` tickets use: `-` bullets,
/// `- [ ]` checklists, ATX headings, blank lines between blocks.
pub fn to_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    write_blocks(blocks, "", &mut out);
    out
}

fn write_blocks(blocks: &[Block], indent: &str, out: &mut String) {
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_block(block, indent, out);
    }
}

fn write_block(block: &Block, indent: &str, out: &mut String) {
    match block {
        Block::Heading(level, inlines) => {
            out.push_str(indent);
            out.push_str(&"#".repeat((*level).clamp(1, 6) as usize));
            out.push(' ');
            out.push_str(&inline_markdown(inlines));
            out.push('\n');
        }
        Block::Paragraph(inlines) => {
            let text = inline_markdown(inlines);
            for line in text.split('\n') {
                out.push_str(indent);
                out.push_str(&escape_line_start(line));
                out.push('\n');
            }
        }
        Block::List { ordered, items } => {
            for (n, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", n + 1)
                } else {
                    "- ".to_string()
                };
                let task = match item.checked {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                let child_indent = format!("{indent}{}", " ".repeat(marker.len()));
                out.push_str(indent);
                out.push_str(&marker);
                out.push_str(task);
                let mut rest: &[Block] = &item.blocks;
                if let Some(Block::Paragraph(inlines)) = item.blocks.first() {
                    let text = inline_markdown(inlines);
                    let mut lines = text.split('\n');
                    out.push_str(lines.next().unwrap_or(""));
                    for line in lines {
                        out.push('\n');
                        out.push_str(&child_indent);
                        out.push_str(line);
                    }
                    rest = &item.blocks[1..];
                }
                // Trim the trailing space of an empty item ("- [ ]").
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
                for b in rest {
                    write_block(b, &child_indent, out);
                }
            }
        }
        Block::Quote(blocks) => {
            let mut inner = String::new();
            write_blocks(blocks, "", &mut inner);
            for line in inner.lines() {
                out.push_str(indent);
                out.push('>');
                if !line.is_empty() {
                    out.push(' ');
                    out.push_str(line);
                }
                out.push('\n');
            }
        }
        Block::Code(code) => {
            out.push_str(indent);
            out.push_str("```\n");
            for line in code.lines() {
                out.push_str(indent);
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(indent);
            out.push_str("```\n");
        }
        Block::Rule => {
            out.push_str(indent);
            out.push_str("---\n");
        }
    }
}

fn inline_markdown(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(&escape_inline(t)),
            Inline::Bold(c) => wrap(&mut out, "**", &inline_markdown(c)),
            Inline::Italic(c) => wrap(&mut out, "*", &inline_markdown(c)),
            Inline::Code(c) => {
                let fence = if c.contains('`') { "`` " } else { "`" };
                out.push_str(fence);
                out.push_str(c);
                out.push_str(&fence.chars().rev().collect::<String>());
            }
            Inline::Link { href, children } => {
                out.push('[');
                out.push_str(&inline_markdown(children));
                out.push_str("](");
                out.push_str(href);
                out.push(')');
            }
            // A CommonMark hard break, so the line break survives a round trip.
            Inline::LineBreak => out.push_str("\\\n"),
        }
    }
    out
}

/// Emphasis markers must hug the text, so surrounding spaces move outside.
fn wrap(out: &mut String, marker: &str, inner: &str) {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        out.push_str(inner);
        return;
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    out.push_str(lead);
    out.push_str(marker);
    out.push_str(trimmed);
    out.push_str(marker);
    out.push_str(trail);
}

/// Escapes only the characters that would otherwise *become* formatting,
/// so text such as `[ - criterion ]` round-trips unchanged.
fn escape_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '_' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A paragraph line that starts like a heading, list item or quote would be
/// read back as one; escape that first character.
fn escape_line_start(line: &str) -> String {
    let t = line.trim_start();
    let starts_block =
        t.starts_with('#') || t.starts_with('>') || t.starts_with("- ") || t.starts_with("+ ") || {
            let digits = t.chars().take_while(char::is_ascii_digit).count();
            digits > 0 && t[digits..].starts_with(". ")
        };
    if starts_block {
        let indent = &line[..line.len() - t.len()];
        format!("{indent}\\{t}")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(md: &str) -> String {
        to_markdown(&parse_markdown(md))
    }

    #[test]
    fn template_body_round_trips() {
        let body = "## Notes\n\n\n## Acceptance criteria\n\n- [ ]\n";
        assert_eq!(
            round_trip(body),
            "## Notes\n\n## Acceptance criteria\n\n- [ ]\n"
        );
    }

    #[test]
    fn users_bracket_criteria_round_trip_unchanged() {
        let body = "## Acceptance criteria\n\n- [ - Tickets can be seen on the board\n- [ - a collapsed column can be re-opened ]\n";
        assert_eq!(round_trip(body), body);
    }

    #[test]
    fn checklists_keep_their_state() {
        let body = "- [ ] open item\n- [x] done item\n";
        let blocks = parse_markdown(body);
        let Block::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items[0].checked, Some(false));
        assert_eq!(items[1].checked, Some(true));
        assert_eq!(to_markdown(&blocks), body);
    }

    #[test]
    fn inline_formatting_round_trips() {
        let body = "Some **bold**, *italic*, `code` and a [link](https://example.com).\n";
        assert_eq!(round_trip(body), body);
    }

    #[test]
    fn numbered_lists_and_headings() {
        let body = "# Title\n\n1. first\n2. second\n";
        assert_eq!(round_trip(body), body);
    }

    #[test]
    fn literal_markdown_characters_are_escaped() {
        let blocks = vec![Block::Paragraph(vec![Inline::Text(
            "# not a heading *or* emphasis".into(),
        )])];
        let md = to_markdown(&blocks);
        assert_eq!(md, "\\# not a heading \\*or\\* emphasis\n");
        assert_eq!(parse_markdown(&md), blocks);
    }

    #[test]
    fn html_is_escaped() {
        let html = to_html(&parse_markdown("<script>alert(1)</script>\n\nA & B <b>\n"));
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(html.contains("A &amp; B &lt;b&gt;"), "{html}");
    }

    #[test]
    fn renders_task_items_with_checkboxes() {
        let html = to_html(&parse_markdown("- [x] done\n- plain\n"));
        assert_eq!(
            html,
            "<ul><li data-task=\"true\"><input type=\"checkbox\" contenteditable=\"false\" checked>done</li><li>plain</li></ul>"
        );
    }

    #[test]
    fn formatting_inside_tight_list_items_survives() {
        let body = "- [x] keep **bold** and *italic* and [a link](https://x.y)\n- plain\n";
        assert_eq!(round_trip(body), body);
    }

    #[test]
    fn line_breaks_survive() {
        let body = "first line\\\nsecond line\n";
        let blocks = parse_markdown(body);
        assert_eq!(
            blocks,
            [Block::Paragraph(vec![
                Inline::Text("first line".into()),
                Inline::LineBreak,
                Inline::Text("second line".into()),
            ])]
        );
        assert_eq!(to_markdown(&blocks), body);
    }

    #[test]
    fn emphasis_markers_hug_text() {
        let blocks = vec![Block::Paragraph(vec![
            Inline::Text("a".into()),
            Inline::Bold(vec![Inline::Text(" b ".into())]),
            Inline::Text("c".into()),
        ])];
        assert_eq!(to_markdown(&blocks), "a **b** c\n");
    }
}
