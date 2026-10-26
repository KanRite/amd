//! The browser half of the rich editor: reading the edited `contenteditable`
//! DOM back into [`rite_ui_model::rich`] blocks, and the toolbar's commands.
//!
//! The editor is filled from `rich::to_html`, but WebKit's editing commands
//! and the user's typing add their own elements (`<div>`, `<b>`, `<span>`,
//! stray `<br>`s), so the reader accepts more than the renderer emits and
//! flattens anything it does not know into plain text rather than losing it.

use rite_ui_model::rich::{Block, Inline, ListItem};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlDocument, HtmlInputElement, Node};

const ELEMENT: u16 = 1;
const TEXT: u16 = 3;

/// Reads the editor's children as blocks.
pub fn read_blocks(root: &Node) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose = Vec::new();
    for node in children(root) {
        match block_of(&node) {
            Some(BlockNode::Block(block)) => {
                flush(&mut loose, &mut blocks);
                blocks.push(block);
            }
            Some(BlockNode::Container) => {
                flush(&mut loose, &mut blocks);
                blocks.extend(read_blocks(&node));
            }
            None => read_inline(&node, &mut loose),
        }
    }
    flush(&mut loose, &mut blocks);
    blocks
}

enum BlockNode {
    Block(Block),
    /// A `<div>`/`<p>` wrapping other blocks: read its children in place.
    Container,
}

fn block_of(node: &Node) -> Option<BlockNode> {
    let el = node.dyn_ref::<Element>()?;
    let tag = el.tag_name().to_ascii_uppercase();
    Some(BlockNode::Block(match tag.as_str() {
        "H1" | "H2" | "H3" | "H4" | "H5" | "H6" => {
            let level = tag.as_bytes()[1] - b'0';
            Block::Heading(level, read_inlines(node))
        }
        "P" | "DIV" => {
            if children(node).iter().any(|c| block_of_tag(c).is_some()) {
                return Some(BlockNode::Container);
            }
            Block::Paragraph(read_inlines(node))
        }
        "UL" | "OL" => Block::List {
            ordered: tag == "OL",
            items: children(node)
                .iter()
                .filter_map(|c| c.dyn_ref::<Element>())
                .filter(|c| c.tag_name().eq_ignore_ascii_case("LI"))
                .map(read_item)
                .collect(),
        },
        "BLOCKQUOTE" => Block::Quote(read_blocks(node)),
        "PRE" => Block::Code(
            node.text_content()
                .unwrap_or_default()
                .replace('\u{a0}', " "),
        ),
        "HR" => Block::Rule,
        _ => return None,
    }))
}

fn block_of_tag(node: &Node) -> Option<()> {
    let el = node.dyn_ref::<Element>()?;
    matches!(
        el.tag_name().to_ascii_uppercase().as_str(),
        "H1" | "H2"
            | "H3"
            | "H4"
            | "H5"
            | "H6"
            | "P"
            | "DIV"
            | "UL"
            | "OL"
            | "BLOCKQUOTE"
            | "PRE"
            | "HR"
    )
    .then_some(())
}

fn read_item(li: &Element) -> ListItem {
    let checked = (li.get_attribute("data-task").as_deref() == Some("true")).then(|| {
        li.query_selector("input[type=checkbox]")
            .ok()
            .flatten()
            .and_then(|i| i.dyn_into::<HtmlInputElement>().ok())
            .is_some_and(|i| i.checked())
    });
    ListItem {
        checked,
        blocks: read_blocks(li),
    }
}

fn flush(loose: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    let inlines = tidy(std::mem::take(loose));
    if !inlines.is_empty() {
        blocks.push(Block::Paragraph(inlines));
    }
}

fn read_inlines(node: &Node) -> Vec<Inline> {
    let mut out = Vec::new();
    for child in children(node) {
        read_inline(&child, &mut out);
    }
    tidy(out)
}

fn read_inline(node: &Node, out: &mut Vec<Inline>) {
    match node.node_type() {
        TEXT => {
            let text = node
                .text_content()
                .unwrap_or_default()
                .replace('\u{a0}', " ");
            if !text.is_empty() {
                out.push(Inline::Text(text));
            }
        }
        ELEMENT => {
            let el: &Element = node.unchecked_ref();
            match el.tag_name().to_ascii_uppercase().as_str() {
                "B" | "STRONG" => push_nonempty(out, Inline::Bold(read_inlines(node))),
                "I" | "EM" => push_nonempty(out, Inline::Italic(read_inlines(node))),
                "CODE" => out.push(Inline::Code(node.text_content().unwrap_or_default())),
                "A" => out.push(Inline::Link {
                    href: el.get_attribute("href").unwrap_or_default(),
                    children: read_inlines(node),
                }),
                "BR" => out.push(Inline::LineBreak),
                "INPUT" => {}
                // <span>, <font> and anything else: keep the text.
                _ => {
                    for child in children(node) {
                        read_inline(&child, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn push_nonempty(out: &mut Vec<Inline>, inline: Inline) {
    match &inline {
        Inline::Bold(c) | Inline::Italic(c) if c.is_empty() => {}
        _ => out.push(inline),
    }
}

/// Merges adjacent text, drops the trailing `<br>` WebKit keeps in a block so
/// it has height, and trims whitespace-only edges.
fn tidy(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (out.last_mut(), inline) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => prev.push_str(&next),
            (_, inline) => out.push(inline),
        }
    }
    while matches!(out.last(), Some(Inline::LineBreak)) {
        out.pop();
    }
    if let Some(Inline::Text(t)) = out.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Inline::Text(t)) = out.last_mut() {
        *t = t.trim_end().to_string();
    }
    out.retain(|i| !matches!(i, Inline::Text(t) if t.is_empty()));
    out
}

fn children(node: &Node) -> Vec<Node> {
    let list = node.child_nodes();
    (0..list.length()).filter_map(|i| list.item(i)).collect()
}

// ---------------------------------------------------------------- toolbar commands

fn document() -> Option<HtmlDocument> {
    web_sys::window()?
        .document()?
        .dyn_into::<HtmlDocument>()
        .ok()
}

/// Runs a built-in editing command on the current selection.
pub fn exec(command: &str, value: Option<&str>) {
    if let Some(doc) = document() {
        let _ = match value {
            Some(v) => doc.exec_command_with_show_ui_and_value(command, false, v),
            None => doc.exec_command(command),
        };
    }
}

/// New blocks typed with Enter become `<p>`, not WebKit's default `<div>`.
pub fn use_paragraphs() {
    exec("defaultParagraphSeparator", Some("p"));
}

/// Toggles the block around the cursor between a heading and a paragraph.
pub fn toggle_heading() {
    let current = document()
        .and_then(|d| d.query_command_value("formatBlock").ok())
        .unwrap_or_default();
    let target = if current.eq_ignore_ascii_case("h2") {
        "<p>"
    } else {
        "<h2>"
    };
    exec("formatBlock", Some(target));
}

/// Turns the list item at the cursor into a checklist item or back, making a
/// bullet list first if the cursor is not in one.
pub fn toggle_checklist() {
    let Some(li) = item_at_cursor().or_else(|| {
        exec("insertUnorderedList", None);
        item_at_cursor()
    }) else {
        return;
    };
    if li.get_attribute("data-task").as_deref() == Some("true") {
        let _ = li.remove_attribute("data-task");
        if let Ok(Some(input)) = li.query_selector("input[type=checkbox]") {
            input.remove();
        }
    } else if let Some(doc) = document() {
        let _ = li.set_attribute("data-task", "true");
        if let Ok(input) = doc.create_element("input") {
            let _ = input.set_attribute("type", "checkbox");
            let _ = input.set_attribute("contenteditable", "false");
            let _ = li.prepend_with_node_1(&input);
        }
    }
}

fn item_at_cursor() -> Option<Element> {
    let selection = web_sys::window()?.get_selection().ok()??;
    let node = selection.anchor_node()?;
    let el = match node.dyn_ref::<Element>() {
        Some(el) => el.clone(),
        None => node.parent_element()?,
    };
    el.closest("li").ok()?
}

/// The selection inside the editor, kept while the link field has focus.
pub fn save_selection() -> Option<web_sys::Range> {
    let selection = web_sys::window()?.get_selection().ok()??;
    (selection.range_count() > 0)
        .then(|| selection.get_range_at(0).ok())
        .flatten()
}

pub fn restore_selection(range: &web_sys::Range) {
    if let Some(selection) = web_sys::window().and_then(|w| w.get_selection().ok().flatten()) {
        let _ = selection.remove_all_ranges();
        let _ = selection.add_range(range);
    }
}
