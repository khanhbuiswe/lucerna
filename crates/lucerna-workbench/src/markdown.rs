//! Markdown for popovers (hover): parsed once into [`MarkdownBlocks`],
//! rendered every frame as gpui elements. Code blocks are highlighted
//! through `lucerna-syntax`; the runs are cached in the blocks.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    AnyElement, App, Context, ElementId, InteractiveText, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledText, TextRun, UnderlineStyle, Window, div, font,
    prelude::*, px,
};
use lucerna_syntax::{Language, LanguageRegistry, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};
use lucerna_text::{Buffer, Point};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::AppState;
use crate::editor_groups::OpenMode;
use crate::layout::color;
use crate::tasks::catch_panic;
use crate::ui::{UI_FONT_FAMILY, UI_FONT_SIZE_SMALL};
use crate::workbench::Workbench;

/// Longer sources are cut here, on a char boundary.
pub const MAX_MARKDOWN_BYTES: usize = 100_000;

/// Tabs in code blocks are shown as this many spaces.
const TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Code {
        /// The fence's language name, as written (first word of the info
        /// string).
        language: Option<String>,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Rule,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Code(String),
    Link { url: String, children: Vec<Inline> },
    SoftBreak,
    HardBreak,
}

/// What a click on a link does, given its URL (see [`open_markdown_link`]).
pub type LinkHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The runs of one code block, for the theme, registry and font they were
/// computed with.
struct CachedRuns {
    theme_generation: u64,
    languages_generation: u64,
    font_family: SharedString,
    runs: Arc<Vec<TextRun>>,
}

/// Parsed markdown: built once, rendered many times.
pub struct MarkdownBlocks {
    blocks: Vec<Block>,
    fallback_language: Option<String>,
    /// By code block, numbered in document order.
    code_runs: Mutex<HashMap<usize, CachedRuns>>,
}

impl std::fmt::Debug for MarkdownBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownBlocks")
            .field("blocks", &self.blocks)
            .field("fallback_language", &self.fallback_language)
            .finish_non_exhaustive()
    }
}

impl MarkdownBlocks {
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The language id of fences that name none (or an unknown one).
    pub fn fallback_language(&self) -> Option<&str> {
        self.fallback_language.as_deref()
    }

    /// The text [`render_markdown`] draws for code block `index` (document
    /// order, nested blocks included) and its highlight runs, which cover
    /// that text exactly. `None` when there is no such block. Computed on
    /// first use per theme and registry, then cached.
    pub fn code_runs(&self, index: usize, cx: &App) -> Option<(SharedString, Arc<Vec<TextRun>>)> {
        let (language, text) = nth_code(&self.blocks, index)?;
        let shown = SharedString::from(text.trim_end_matches('\n').to_owned());
        let app = cx.global::<AppState>();
        let family = code_font_family(cx);
        let mut cache = self.code_runs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.get(&index)
            && c.theme_generation == app.theme_generation
            && c.languages_generation == app.languages_generation
            && c.font_family == family
        {
            return Some((shown, c.runs.clone()));
        }
        let spans = resolve_language(
            &app.languages,
            language.as_deref(),
            self.fallback_language(),
        )
        .map(|l| {
            highlight(&shown, &l, &app.languages, &|c| {
                app.theme.style_for(c).is_some()
            })
        })
        .unwrap_or_default();
        let ui = &app.theme.ui;
        let base = font(family.clone());
        let plain = TextRun {
            len: 0,
            font: base.clone(),
            color: color(ui.editor_foreground).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = Arc::new(code_runs(&shown, &spans, &app.theme, &plain));
        cache.insert(
            index,
            CachedRuns {
                theme_generation: app.theme_generation,
                languages_generation: app.languages_generation,
                font_family: family,
                runs: runs.clone(),
            },
        );
        Some((shown, runs))
    }
}

fn nth_code(blocks: &[Block], index: usize) -> Option<(Option<String>, String)> {
    fn walk(blocks: &[Block], seen: &mut usize, index: usize) -> Option<(Option<String>, String)> {
        for block in blocks {
            match block {
                Block::Code { language, text } => {
                    if *seen == index {
                        return Some((language.clone(), text.clone()));
                    }
                    *seen += 1;
                }
                Block::Quote(inner) => {
                    if let Some(found) = walk(inner, seen, index) {
                        return Some(found);
                    }
                }
                Block::List { items, .. } => {
                    for item in items {
                        if let Some(found) = walk(item, seen, index) {
                            return Some(found);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(blocks, &mut 0, index)
}

fn code_font_family(cx: &App) -> SharedString {
    cx.global::<AppState>()
        .settings
        .get::<String>("editor.fontFamily", None)
        .unwrap_or_else(|| crate::fonts::EDITOR_FONT_FAMILY.into())
        .into()
}

/// Capture spans of `text` in `language`; empty when the parse does not
/// finish within [`SYNC_PARSE_BUDGET`] or the grammar fails.
fn highlight(
    text: &str,
    language: &Arc<Language>,
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> Vec<(Range<usize>, Arc<str>)> {
    catch_panic("markdown highlight", || {
        let buffer = Buffer::from_bytes(text.as_bytes(), true).ok()?;
        let snapshot = buffer.snapshot();
        let mut syntax = SyntaxState::new(language.clone(), registry.clone()).ok()?;
        if syntax.parse(&snapshot, Some(SYNC_PARSE_BUDGET)) != ParseOutcome::Done {
            return None;
        }
        let spans = syntax.highlights(&snapshot, 0..snapshot.line_count(), styled);
        Some(spans.into_iter().map(|s| (s.range, s.capture)).collect())
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Runs covering `text`: `plain` where no span applies, else the style of
/// the innermost (last) span covering the byte.
fn code_runs(
    text: &str,
    spans: &[(Range<usize>, Arc<str>)],
    theme: &lucerna_theme::Theme,
    plain: &TextRun,
) -> Vec<TextRun> {
    let mut owner: Vec<Option<u32>> = vec![None; text.len()];
    for (i, (range, capture)) in spans.iter().enumerate() {
        let range = range.start..range.end.min(text.len());
        if range.start >= range.end
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
            || theme.style_for(capture).is_none()
        {
            continue;
        }
        owner[range].fill(Some(i as u32));
    }
    let mut runs: Vec<TextRun> = Vec::new();
    let mut last: Option<Option<u32>> = None;
    for (byte, _) in text.char_indices() {
        let own = owner[byte];
        let width = text[byte..].chars().next().map_or(0, char::len_utf8);
        if last == Some(own) {
            if let Some(run) = runs.last_mut() {
                run.len += width;
            }
            continue;
        }
        last = Some(own);
        let mut run = plain.clone();
        run.len = width;
        if let Some(style) = own.and_then(|i| theme.style_for(&spans[i as usize].1)) {
            if let Some(c) = style.color {
                run.color = color(c).into();
            }
            if style.bold {
                run.font = run.font.bold();
            }
            if style.italic {
                run.font = run.font.italic();
            }
        }
        runs.push(run);
    }
    runs
}

/// The language of a fence: its own name or id (also VS Code ids such as
/// `ts` or `bash`), else `fallback` (the file's language id).
pub fn resolve_language(
    registry: &LanguageRegistry,
    fence: Option<&str>,
    fallback: Option<&str>,
) -> Option<Arc<Language>> {
    let by_alias = |name: &str| {
        let id = match name.to_ascii_lowercase().as_str() {
            "bash" | "sh" | "zsh" | "shell" | "console" => "shellscript",
            "ts" => "typescript",
            "js" | "node" => "javascript",
            "py" => "python",
            "yml" => "yaml",
            "rs" => "rust",
            _ => return None,
        };
        registry.language_by_name(id)
    };
    let find = |name: &str| registry.language_by_name(name).or_else(|| by_alias(name));
    fence
        .filter(|f| !f.is_empty())
        .and_then(find)
        .or_else(|| fallback.and_then(find))
}

// ---- Parsing ----

pub fn parse_markdown(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks {
    // pulldown-cmark ends a fence left open by the cut at the end of the
    // input, so the cut text still forms one code block.
    let mut cut = source.len().min(MAX_MARKDOWN_BYTES);
    while !source.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut builder = Builder::default();
    for event in Parser::new(&source[..cut]) {
        builder.event(event);
    }
    MarkdownBlocks {
        blocks: builder.finish(),
        fallback_language: fallback_language.map(str::to_owned),
        code_runs: Mutex::new(HashMap::new()),
    }
}

enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Item(Vec<Block>),
}

enum Span {
    /// The inlines of a paragraph or heading.
    Base,
    Strong,
    Emph,
    Link(String),
    /// Images show their alt text.
    Image,
}

#[derive(Default)]
struct Builder {
    frames: Vec<Frame>,
    spans: Vec<(Span, Vec<Inline>)>,
    /// Text that came without a paragraph (tight list items) is collected
    /// in a `Span::Base` that no `Start(Paragraph)` opened.
    implicit: bool,
    code: Option<(Option<String>, String)>,
}

impl Builder {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.code {
                Some((_, code)) => code.push_str(&text.replace('\t', &" ".repeat(TAB_WIDTH))),
                None => self.inline(Inline::Text(text.into_string())),
            },
            Event::Code(code) => self.inline(Inline::Code(code.into_string())),
            Event::SoftBreak => self.inline(Inline::SoftBreak),
            Event::HardBreak => self.inline(Inline::HardBreak),
            Event::Rule => {
                self.flush_implicit();
                self.block(Block::Rule);
            }
            // Raw HTML, footnotes, math, task markers: not shown.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush_implicit();
                self.spans.push((Span::Base, Vec::new()));
            }
            Tag::Heading { .. } => {
                self.flush_implicit();
                self.spans.push((Span::Base, Vec::new()));
            }
            Tag::BlockQuote(_) => {
                self.flush_implicit();
                self.frames.push(Frame::Quote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.flush_implicit();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c == ',' || c == '{' || c.is_whitespace())
                        .find(|w| !w.is_empty())
                        .map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((language, String::new()));
            }
            Tag::List(first) => {
                self.flush_implicit();
                self.frames.push(Frame::List {
                    ordered: first.is_some(),
                    items: Vec::new(),
                });
            }
            Tag::Item => self.frames.push(Frame::Item(Vec::new())),
            Tag::Strong => self.spans.push((Span::Strong, Vec::new())),
            Tag::Emphasis => self.spans.push((Span::Emph, Vec::new())),
            Tag::Link { dest_url, .. } => self
                .spans
                .push((Span::Link(dest_url.into_string()), Vec::new())),
            Tag::Image { .. } => self.spans.push((Span::Image, Vec::new())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if let Some((_, inlines)) = self.spans.pop()
                    && !inlines.is_empty()
                {
                    self.block(Block::Paragraph(inlines));
                }
            }
            TagEnd::Heading(level) => {
                if let Some((_, inlines)) = self.spans.pop() {
                    self.block(Block::Heading(level as u8, inlines));
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush_implicit();
                if let Some(Frame::Quote(blocks)) = self.frames.pop() {
                    self.block(Block::Quote(blocks));
                }
            }
            TagEnd::CodeBlock => {
                if let Some((language, text)) = self.code.take() {
                    self.block(Block::Code { language, text });
                }
            }
            TagEnd::List(_) => {
                if let Some(Frame::List { ordered, items }) = self.frames.pop() {
                    self.block(Block::List { ordered, items });
                }
            }
            TagEnd::Item => {
                self.flush_implicit();
                if let Some(Frame::Item(blocks)) = self.frames.pop()
                    && let Some(Frame::List { items, .. }) = self.frames.last_mut()
                {
                    items.push(blocks);
                }
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link | TagEnd::Image => {
                if let Some((span, children)) = self.spans.pop() {
                    match span {
                        Span::Strong => self.inline(Inline::Strong(children)),
                        Span::Emph => self.inline(Inline::Emph(children)),
                        Span::Link(url) => self.inline(Inline::Link { url, children }),
                        Span::Image | Span::Base => {
                            for child in children {
                                self.inline(child);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn inline(&mut self, inline: Inline) {
        if self.spans.is_empty() {
            self.spans.push((Span::Base, Vec::new()));
            self.implicit = true;
        }
        let Some((_, items)) = self.spans.last_mut() else {
            return;
        };
        match (items.last_mut(), inline) {
            (Some(Inline::Text(last)), Inline::Text(next)) => last.push_str(&next),
            (_, inline) => items.push(inline),
        }
    }

    /// Ends the paragraph that a tight list item's loose text formed.
    fn flush_implicit(&mut self) {
        if !self.implicit {
            return;
        }
        self.implicit = false;
        if let Some((_, inlines)) = self.spans.pop()
            && !inlines.is_empty()
        {
            self.block(Block::Paragraph(inlines));
        }
    }

    fn block(&mut self, block: Block) {
        match self.frames.last_mut() {
            Some(Frame::Quote(blocks) | Frame::Item(blocks) | Frame::Root(blocks)) => {
                blocks.push(block)
            }
            _ => self.frames.insert(0, Frame::Root(vec![block])),
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush_implicit();
        match self.frames.into_iter().next() {
            Some(Frame::Root(blocks)) => blocks,
            _ => Vec::new(),
        }
    }
}

// ---- Links ----

/// What a link opens.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkTarget {
    Web(String),
    File { path: PathBuf, point: Point },
}

/// `http(s)` URLs open in the browser; `file:///abs/path#L10` (or
/// `#L10,5`, 1-based) opens the file at that place. Anything else is not
/// a link Lucerna follows.
pub fn link_target(url: &str) -> Option<LinkTarget> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(LinkTarget::Web(url.to_owned())),
        "file" => {
            let path = parsed.to_file_path().ok()?;
            let point = parsed
                .fragment()
                .map_or(Point { row: 0, column: 0 }, line_fragment);
            Some(LinkTarget::File { path, point })
        }
        _ => None,
    }
}

/// `L10`, `L10,5`, `L10:5` or `L10C5` as a 0-based point.
fn line_fragment(fragment: &str) -> Point {
    let zero = Point { row: 0, column: 0 };
    let Some(rest) = fragment.strip_prefix('L') else {
        return zero;
    };
    let digits = |s: &str| -> (u32, usize) {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        (s[..end].parse().unwrap_or(0), end)
    };
    let (row, end) = digits(rest);
    let column = rest[end..]
        .strip_prefix([',', ':', 'C'])
        .map_or(0, |c| digits(c).0);
    Point {
        row: row.saturating_sub(1),
        column: column.saturating_sub(1),
    }
}

/// Follows a link of rendered markdown: web links open in the browser,
/// file links in a tab (a preview tab) at their line.
pub fn open_markdown_link(url: &str, wb: &mut Workbench, cx: &mut Context<Workbench>) {
    match link_target(url) {
        Some(LinkTarget::Web(url)) => cx.open_url(&url),
        Some(LinkTarget::File { path, point }) => {
            wb.jump_to(Arc::from(path), point, OpenMode::Preview, cx)
        }
        None => {}
    }
}

// ---- Rendering ----

/// Renders `blocks`; clicking a web link opens the browser. Use
/// [`render_markdown_with`] to open file links too.
pub fn render_markdown(blocks: &MarkdownBlocks, cx: &App) -> AnyElement {
    render_markdown_with(
        blocks,
        Rc::new(|url, _, cx| {
            if let Some(LinkTarget::Web(url)) = link_target(url) {
                cx.open_url(&url);
            }
        }),
        cx,
    )
}

/// Renders `blocks`; `on_link` gets the URL of a clicked link.
pub fn render_markdown_with(blocks: &MarkdownBlocks, on_link: LinkHandler, cx: &App) -> AnyElement {
    let mut renderer = Renderer {
        blocks,
        cx,
        on_link,
        texts: 0,
        codes: 0,
    };
    let children = renderer.blocks(&blocks.blocks, UI_FONT_SIZE_SMALL, false);
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .min_w_0()
        .font_family(UI_FONT_FAMILY)
        .text_size(px(UI_FONT_SIZE_SMALL))
        .text_color(color(cx.global::<AppState>().theme.ui.text))
        .children(children)
        .into_any_element()
}

struct Renderer<'a> {
    blocks: &'a MarkdownBlocks,
    cx: &'a App,
    on_link: LinkHandler,
    /// Ids for the clickable text elements.
    texts: usize,
    /// Code blocks drawn so far: the next one's index.
    codes: usize,
}

/// The font and color a run of inlines is drawn with.
#[derive(Clone, Copy)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
    muted: bool,
}

/// Heading sizes by level: slightly above body text.
fn heading_size(level: u8) -> f32 {
    match level {
        1 => 18.,
        2 => 16.,
        3 => 15.,
        _ => 14.,
    }
}

impl Renderer<'_> {
    fn blocks(&mut self, blocks: &[Block], size: f32, muted: bool) -> Vec<AnyElement> {
        blocks
            .iter()
            .map(|block| self.block(block, size, muted))
            .collect()
    }

    fn block(&mut self, block: &Block, size: f32, muted: bool) -> AnyElement {
        let ui = &self.cx.global::<AppState>().theme.ui;
        match block {
            Block::Paragraph(inlines) => self.text(inlines, size, false, muted),
            Block::Heading(level, inlines) => self.text(inlines, heading_size(*level), true, muted),
            Block::Code { .. } => self.code(),
            Block::List { ordered, items } => div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(items.iter().enumerate().map(|(i, item)| {
                    let marker = if *ordered {
                        format!("{}.", i + 1)
                    } else {
                        "•".to_owned()
                    };
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(div().flex_none().child(marker))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .children(self.blocks(item, size, muted)),
                        )
                }))
                .into_any_element(),
            Block::Quote(inner) => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pl(px(8.))
                .border_l_2()
                .border_color(color(ui.border))
                .text_color(color(ui.text_muted))
                .children(self.blocks(inner, size, true))
                .into_any_element(),
            Block::Rule => div()
                .h(px(1.))
                .w_full()
                .my(px(4.))
                .bg(color(ui.border))
                .into_any_element(),
        }
    }

    /// One text element for a paragraph or heading.
    fn text(&mut self, inlines: &[Inline], size: f32, bold: bool, muted: bool) -> AnyElement {
        let mut flat = Flat::default();
        let style = InlineStyle {
            bold,
            italic: false,
            code: false,
            link: false,
            muted,
        };
        self.flatten(inlines, style, None, &mut flat);
        if flat.text.is_empty() {
            return div().into_any_element();
        }
        let styled = StyledText::new(flat.text).with_runs(flat.runs);
        let el = div().text_size(px(size));
        if flat.links.is_empty() {
            return el.child(styled).into_any_element();
        }
        self.texts += 1;
        let (ranges, urls): (Vec<_>, Vec<_>) = flat.links.into_iter().unzip();
        let on_link = self.on_link.clone();
        el.child(
            InteractiveText::new(ElementId::from(("md-text", self.texts)), styled)
                .on_click(ranges, move |ix, window, cx| on_link(&urls[ix], window, cx)),
        )
        .into_any_element()
    }

    fn flatten(&self, inlines: &[Inline], style: InlineStyle, link: Option<&str>, out: &mut Flat) {
        let ui = &self.cx.global::<AppState>().theme.ui;
        let push = |out: &mut Flat, text: &str, style: InlineStyle| {
            if text.is_empty() {
                return;
            }
            let family: SharedString = if style.code {
                code_font_family(self.cx)
            } else {
                UI_FONT_FAMILY.into()
            };
            let mut f = font(family);
            if style.bold {
                f = f.bold();
            }
            if style.italic {
                f = f.italic();
            }
            let text_color = if style.link {
                ui.link_text_hover
            } else if style.muted {
                ui.text_muted
            } else {
                ui.text
            };
            let start = out.text.len();
            out.text.push_str(text);
            out.runs.push(TextRun {
                len: text.len(),
                font: f,
                color: color(text_color).into(),
                background_color: style.code.then(|| color(ui.editor_background).into()),
                underline: style.link.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                strikethrough: None,
            });
            if let Some(url) = link {
                match out.links.last_mut() {
                    Some((range, last)) if range.end == start && last == url => {
                        range.end = out.text.len()
                    }
                    _ => out.links.push((start..out.text.len(), url.to_owned())),
                }
            }
        };
        for inline in inlines {
            match inline {
                Inline::Text(t) => push(out, t, style),
                Inline::Code(t) => push(
                    out,
                    t,
                    InlineStyle {
                        code: true,
                        ..style
                    },
                ),
                Inline::SoftBreak => push(out, " ", style),
                Inline::HardBreak => push(out, "\n", style),
                Inline::Strong(children) => self.flatten(
                    children,
                    InlineStyle {
                        bold: true,
                        ..style
                    },
                    link,
                    out,
                ),
                Inline::Emph(children) => self.flatten(
                    children,
                    InlineStyle {
                        italic: true,
                        ..style
                    },
                    link,
                    out,
                ),
                Inline::Link { url, children } => self.flatten(
                    children,
                    InlineStyle {
                        link: true,
                        ..style
                    },
                    Some(url),
                    out,
                ),
            }
        }
    }

    /// The next code block: code font, editor background, no wrap, scrolls
    /// sideways.
    fn code(&mut self) -> AnyElement {
        let index = self.codes;
        self.codes += 1;
        let ui = &self.cx.global::<AppState>().theme.ui;
        let (text, runs) = self.blocks.code_runs(index, self.cx).unwrap_or_default();
        let code = div()
            .id(("md-code-scroll", index))
            .w_full()
            .overflow_x_scroll()
            .rounded(px(4.))
            .bg(color(ui.editor_background))
            .child(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .whitespace_nowrap()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .child(StyledText::new(text).with_runs(runs.to_vec())),
            );
        div()
            .debug_selector(move || format!("md-code-{index}"))
            .w_full()
            .child(code)
            .into_any_element()
    }
}

#[derive(Default)]
struct Flat {
    text: String,
    runs: Vec<TextRun>,
    /// Byte ranges of `text` and their URLs.
    links: Vec<(Range<usize>, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Point;
    use std::path::PathBuf;

    fn text(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    fn parse(s: &str) -> Vec<Block> {
        parse_markdown(s, None).blocks().to_vec()
    }

    #[test]
    fn parses_fenced_code_with_language() {
        assert_eq!(
            parse("```rust\nfn main() {}\n```"),
            vec![Block::Code {
                language: Some("rust".into()),
                text: "fn main() {}\n".into()
            }]
        );
        assert_eq!(
            parse("```rust,ignore\nx\n```"),
            vec![Block::Code {
                language: Some("rust".into()),
                text: "x\n".into()
            }]
        );
    }

    #[test]
    fn marked_string_language_fallback() {
        let blocks = parse_markdown("```\nlet x = 1;\n```", Some("rust"));
        assert_eq!(
            blocks.blocks(),
            [Block::Code {
                language: None,
                text: "let x = 1;\n".into()
            }]
        );
        assert_eq!(blocks.fallback_language(), Some("rust"));

        let registry = LanguageRegistry::bundled();
        let id = |fence, fallback| {
            resolve_language(&registry, fence, fallback).map(|l| l.id.to_string())
        };
        assert_eq!(id(None, Some("rust")).as_deref(), Some("rust"));
        assert_eq!(id(Some("klingon"), Some("rust")).as_deref(), Some("rust"));
        assert_eq!(id(Some("python"), Some("rust")).as_deref(), Some("python"));
        assert_eq!(id(Some("Rust"), None).as_deref(), Some("rust"));
        assert_eq!(id(Some("klingon"), None), None);
        assert_eq!(id(None, None), None);
        for (alias, want) in [
            ("ts", "typescript"),
            ("js", "javascript"),
            ("py", "python"),
            ("sh", "shellscript"),
            ("bash", "shellscript"),
            ("yml", "yaml"),
            ("json", "json"),
            ("toml", "toml"),
            ("go", "go"),
            ("html", "html"),
            ("css", "css"),
        ] {
            assert_eq!(id(Some(alias), None).as_deref(), Some(want), "{alias}");
        }
    }

    #[test]
    fn truncation_closes_fence() {
        let source = format!("```rust\n{}", "let a = 1;\n".repeat(20_000));
        assert!(source.len() > MAX_MARKDOWN_BYTES);
        let blocks = parse(&source);
        let [Block::Code { language, text }] = &blocks[..] else {
            panic!("{blocks:?}");
        };
        assert_eq!(language.as_deref(), Some("rust"));
        assert!(text.len() <= MAX_MARKDOWN_BYTES && text.len() > MAX_MARKDOWN_BYTES - 100);

        // The cut never splits a char.
        let source = "é".repeat(MAX_MARKDOWN_BYTES);
        let blocks = parse(&source);
        let [Block::Paragraph(inlines)] = &blocks[..] else {
            panic!("{blocks:?}");
        };
        let Inline::Text(t) = &inlines[0] else {
            panic!()
        };
        assert_eq!(t.len(), MAX_MARKDOWN_BYTES);
    }

    #[test]
    fn nested_list() {
        let blocks = parse("- a\n  - b\n  - c\n- d\n");
        assert_eq!(
            blocks,
            vec![Block::List {
                ordered: false,
                items: vec![
                    vec![
                        Block::Paragraph(vec![text("a")]),
                        Block::List {
                            ordered: false,
                            items: vec![
                                vec![Block::Paragraph(vec![text("b")])],
                                vec![Block::Paragraph(vec![text("c")])],
                            ],
                        },
                    ],
                    vec![Block::Paragraph(vec![text("d")])],
                ],
            }]
        );
        let ordered = parse("1. x\n2. y\n");
        assert!(matches!(&ordered[..], [Block::List { ordered: true, items }] if items.len() == 2));
    }

    #[test]
    fn link_inline() {
        assert_eq!(
            parse("see [the **docs**](https://x.dev/a) now"),
            vec![Block::Paragraph(vec![
                text("see "),
                Inline::Link {
                    url: "https://x.dev/a".into(),
                    children: vec![text("the "), Inline::Strong(vec![text("docs")])],
                },
                text(" now"),
            ])]
        );
    }

    #[test]
    fn headings_quotes_rules_and_inline_code() {
        assert_eq!(
            parse("## Title\n\n> quoted *em*\n\n---\n\nuse `x` here\\\nnext\nline"),
            vec![
                Block::Heading(2, vec![text("Title")]),
                Block::Quote(vec![Block::Paragraph(vec![
                    text("quoted "),
                    Inline::Emph(vec![text("em")]),
                ])]),
                Block::Rule,
                Block::Paragraph(vec![
                    text("use "),
                    Inline::Code("x".into()),
                    text(" here"),
                    Inline::HardBreak,
                    text("next"),
                    Inline::SoftBreak,
                    text("line"),
                ]),
            ]
        );
    }

    #[test]
    fn plain_text_and_raw_html() {
        assert_eq!(
            parse("just text"),
            vec![Block::Paragraph(vec![text("just text")])]
        );
        assert_eq!(parse(""), Vec::<Block>::new());
        // Raw HTML is dropped, its text content stays.
        assert_eq!(
            parse("a <b>bold</b> c"),
            vec![Block::Paragraph(vec![text("a bold c")])]
        );
    }

    #[test]
    fn link_targets() {
        assert_eq!(
            link_target("https://a.dev/x"),
            Some(LinkTarget::Web("https://a.dev/x".into()))
        );
        assert_eq!(
            link_target("http://a.dev"),
            Some(LinkTarget::Web("http://a.dev".into()))
        );
        let file = |path: &str, row, column| {
            Some(LinkTarget::File {
                path: PathBuf::from(path),
                point: Point { row, column },
            })
        };
        assert_eq!(link_target("file:///a/b.rs#L10"), file("/a/b.rs", 9, 0));
        assert_eq!(link_target("file:///a/b.rs#L10,5"), file("/a/b.rs", 9, 4));
        assert_eq!(link_target("file:///a/b.rs"), file("/a/b.rs", 0, 0));
        assert_eq!(
            link_target("file:///a/my%20f.rs#L1"),
            file("/a/my f.rs", 0, 0)
        );
        assert_eq!(link_target("file:///a/b.rs#L0"), file("/a/b.rs", 0, 0));
        assert_eq!(link_target("javascript:alert(1)"), None);
        assert_eq!(link_target("command:foo"), None);
        assert_eq!(link_target("#anchor"), None);
        assert_eq!(link_target("relative/path.md"), None);
    }
}
