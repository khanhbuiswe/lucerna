//! Markdown for popovers (hover): parsed once into [`MarkdownBlocks`],
//! rendered every frame as gpui elements. Code blocks are highlighted
//! through `lucerna-syntax`; the runs are cached in the blocks.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    AnyElement, App, Context, ElementId, FontWeight, InteractiveText, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText, TextRun,
    UnderlineStyle, Window, div, font, prelude::*, px, relative, svg,
};
use lucerna_syntax::{Language, LanguageRegistry, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};
use lucerna_text::{Buffer, Point};
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag,
    TagEnd,
};

use crate::AppState;
use crate::editor_groups::OpenMode;
use crate::fonts::EDITOR_FONT_FAMILY;
use crate::icons;
use crate::layout::color;
use crate::tasks::catch_panic;
use crate::ui::{GRID, UI_FONT_FAMILY, UI_FONT_SIZE_SMALL, checkbox, tooltip};
use crate::workbench::Workbench;

/// Longer sources are cut here, on a char boundary.
pub const MAX_MARKDOWN_BYTES: usize = 100_000;

/// Containers (quotes, lists) nested deeper than this are merged into
/// their parent: pulldown-cmark has no limit, and rendering, dropping
/// and lookups recurse per level.
const MAX_BLOCK_DEPTH: usize = 16;
/// Same for emphasis and links.
const MAX_INLINE_DEPTH: usize = 32;

/// The pulldown-cmark extensions on, for hover and the preview alike.
/// Front matter is parsed so it can be left out.
pub const PARSE_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS)
    .union(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS)
    .union(Options::ENABLE_GFM);

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
    /// `start` is the first number of an ordered list; `None` for a
    /// bullet list.
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    /// A GitHub alert: `> [!NOTE]` and its siblings.
    Alert {
        kind: AlertKind,
        blocks: Vec<Block>,
    },
    Table {
        aligns: Vec<Align>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A paragraph that holds nothing but one image.
    Image {
        url: String,
        alt: String,
    },
    Rule,
}

/// One list item: `checked` is set for task list items (`- [x]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// The alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// The kind of a GitHub alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AlertKind {
    /// The bold title of the alert.
    pub fn title(self) -> &'static str {
        match self {
            AlertKind::Note => "Note",
            AlertKind::Tip => "Tip",
            AlertKind::Important => "Important",
            AlertKind::Warning => "Warning",
            AlertKind::Caution => "Caution",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Strike(Vec<Inline>),
    Code(String),
    Link { url: String, children: Vec<Inline> },
    Image { url: String, alt: String },
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
    text: SharedString,
    runs: Arc<Vec<TextRun>>,
}

/// Parsed markdown: built once, rendered many times.
pub struct MarkdownBlocks {
    blocks: Vec<Block>,
    /// Source lines of each root block (0-based, end excluded), parallel
    /// to `blocks`.
    root_lines: Vec<Range<u32>>,
    /// Lines of the source.
    line_count: u32,
    /// For each root block, the index of its first code block.
    code_starts: Vec<usize>,
    /// For each root block, the index of its first image.
    image_starts: Vec<usize>,
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

    /// The source lines of each root block, parallel to [`Self::blocks`]
    /// (0-based, the end line not included).
    pub fn root_lines(&self) -> &[Range<u32>] {
        &self.root_lines
    }

    /// The number of lines of the source: its `\n` count, plus one when
    /// the text does not end with one.
    pub fn line_count(&self) -> u32 {
        self.line_count
    }

    /// The `code_runs` index of the first code block in root block `root`
    /// (nested blocks included); the total count when `root` is past the
    /// end.
    pub fn code_index_start(&self, root: usize) -> usize {
        self.code_starts
            .get(root)
            .copied()
            .unwrap_or_else(|| count_code(&self.blocks))
    }

    /// The index of the first image in root block `root`, in document
    /// order (nested blocks, table cells and links included); the total
    /// count when `root` is past the end.
    pub fn image_index_start(&self, root: usize) -> usize {
        self.image_starts
            .get(root)
            .copied()
            .unwrap_or_else(|| count_images(&self.blocks))
    }

    /// The language id of fences that name none (or an unknown one).
    pub fn fallback_language(&self) -> Option<&str> {
        self.fallback_language.as_deref()
    }

    /// The text [`render_markdown`] draws for code block `index` (document
    /// order, nested blocks included) and its highlight runs, which cover
    /// that text exactly. `None` when there is no such block. Computed on
    /// first use per theme, registry and font, then cached. The code is
    /// set in `family` (the window's `editor.fontFamily`).
    pub fn code_runs(
        &self,
        index: usize,
        family: &SharedString,
        cx: &App,
    ) -> Option<(SharedString, Arc<Vec<TextRun>>)> {
        let app = cx.global::<AppState>();
        let family = family.clone();
        let mut cache = self.code_runs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.get(&index)
            && c.theme_generation == app.theme_generation
            && c.languages_generation == app.languages_generation
            && c.font_family == family
        {
            return Some((c.text.clone(), c.runs.clone()));
        }
        let (language, text) = nth_code(&self.blocks, index)?;
        let shown = SharedString::from(text.trim_end_matches('\n').to_owned());
        let spans = resolve_language(&app.languages, language, self.fallback_language())
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
                text: shown.clone(),
                runs: runs.clone(),
            },
        );
        Some((shown, runs))
    }
}

fn nth_code(blocks: &[Block], index: usize) -> Option<(Option<&str>, &str)> {
    fn walk<'a>(
        blocks: &'a [Block],
        seen: &mut usize,
        index: usize,
    ) -> Option<(Option<&'a str>, &'a str)> {
        for block in blocks {
            match block {
                Block::Code { language, text } => {
                    if *seen == index {
                        return Some((language.as_deref(), text));
                    }
                    *seen += 1;
                }
                Block::Quote(inner) | Block::Alert { blocks: inner, .. } => {
                    if let Some(found) = walk(inner, seen, index) {
                        return Some(found);
                    }
                }
                Block::List { items, .. } => {
                    for item in items {
                        if let Some(found) = walk(&item.blocks, seen, index) {
                            return Some(found);
                        }
                    }
                }
                // Table cells hold inlines only.
                _ => {}
            }
        }
        None
    }
    walk(blocks, &mut 0, index)
}

/// The number of code blocks in `blocks`, nested ones included.
fn count_code(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Code { .. } => 1,
            Block::Quote(inner) | Block::Alert { blocks: inner, .. } => count_code(inner),
            Block::List { items, .. } => items.iter().map(|i| count_code(&i.blocks)).sum(),
            _ => 0,
        })
        .sum()
}

/// The number of images in `blocks`.
fn count_images(blocks: &[Block]) -> usize {
    let mut count = 0;
    visit_image_urls(blocks, &mut |_| count += 1);
    count
}

/// Calls `f` with the URL of each image in `blocks`, in the order they are
/// drawn (nested blocks, table cells and links included).
pub(crate) fn visit_image_urls(blocks: &[Block], f: &mut impl FnMut(&str)) {
    fn inline(inlines: &[Inline], f: &mut impl FnMut(&str)) {
        for i in inlines {
            match i {
                Inline::Image { url, .. } => f(url),
                Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => inline(c, f),
                Inline::Link { children, .. } => inline(children, f),
                _ => {}
            }
        }
    }
    for block in blocks {
        match block {
            Block::Image { url, .. } => f(url),
            Block::Paragraph(i) | Block::Heading(_, i) => inline(i, f),
            Block::Quote(inner) | Block::Alert { blocks: inner, .. } => visit_image_urls(inner, f),
            Block::List { items, .. } => items.iter().for_each(|i| visit_image_urls(&i.blocks, f)),
            Block::Table { head, rows, .. } => head
                .iter()
                .chain(rows.iter().flatten())
                .for_each(|cell| inline(cell, f)),
            Block::Code { .. } | Block::Rule => {}
        }
    }
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

/// Parses `source` cut at [`MAX_MARKDOWN_BYTES`] (hover).
pub fn parse_markdown(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks {
    // pulldown-cmark ends a fence left open by the cut at the end of the
    // input, so the cut text still forms one code block.
    let mut cut = source.len().min(MAX_MARKDOWN_BYTES);
    while !source.is_char_boundary(cut) {
        cut -= 1;
    }
    parse(&source[..cut], fallback_language)
}

/// Parses all of `source`: the caller limits its size (the preview).
pub fn parse_markdown_document(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks {
    parse(source, fallback_language)
}

fn parse(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks {
    let mut builder = Builder::default();
    // Open block tags: a root block ends when this returns to 0.
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(source, PARSE_OPTIONS).into_offset_iter() {
        match &event {
            Event::Start(tag) if !is_inline_tag(tag) => {
                if depth == 0 {
                    builder.root_range = range.clone();
                }
                depth += 1;
            }
            Event::End(tag) if !is_inline_end(tag) => depth = depth.saturating_sub(1),
            Event::Rule if depth == 0 => builder.root_range = range.clone(),
            _ => {}
        }
        builder.event(event);
    }
    let (blocks, root_ranges) = builder.finish();
    let newlines: Vec<usize> = source.match_indices('\n').map(|(i, _)| i).collect();
    let line_of = |byte: usize| newlines.partition_point(|&n| n < byte) as u32;
    let root_lines = root_ranges
        .iter()
        .map(|r| {
            let start = line_of(r.start);
            let last = line_of(r.end.saturating_sub(1).max(r.start));
            start..(last + 1)
        })
        .collect();
    let mut total = 0;
    let code_starts = blocks
        .iter()
        .map(|b| {
            let start = total;
            total += count_code(std::slice::from_ref(b));
            start
        })
        .collect();
    let mut total = 0;
    let image_starts = blocks
        .iter()
        .map(|b| {
            let start = total;
            total += count_images(std::slice::from_ref(b));
            start
        })
        .collect();
    let line_count =
        newlines.len() as u32 + u32::from(!source.is_empty() && !source.ends_with('\n'));
    MarkdownBlocks {
        blocks,
        root_lines,
        line_count,
        code_starts,
        image_starts,
        fallback_language: fallback_language.map(str::to_owned),
        code_runs: Mutex::new(HashMap::new()),
    }
}

fn is_inline_tag(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn is_inline_end(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>, Option<AlertKind>),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Item {
        checked: Option<bool>,
        blocks: Vec<Block>,
    },
    Table {
        aligns: Vec<Align>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        /// The cells of the row being read.
        row: Vec<Vec<Inline>>,
    },
}

enum Span {
    /// The inlines of a paragraph or heading.
    Base,
    Strong,
    Emph,
    Strike,
    Link(String),
    Image(String),
    /// The inlines of a table cell.
    Cell,
}

#[derive(Default)]
struct Builder {
    frames: Vec<Frame>,
    spans: Vec<(Span, Vec<Inline>)>,
    /// Text that came without a paragraph (tight list items) is collected
    /// in a `Span::Base` that no `Start(Paragraph)` opened.
    implicit: bool,
    code: Option<(Option<String>, String)>,
    /// For each open container tag: whether it opened a frame (false when
    /// past [`MAX_BLOCK_DEPTH`], its content then joins the parent).
    container_real: Vec<bool>,
    /// The same for open emphasis, strong, link and image tags.
    span_real: Vec<bool>,
    /// Inside front matter, whose text is dropped.
    metadata: bool,
    /// Byte range of the root block being read.
    root_range: Range<usize>,
    /// Byte ranges of the root blocks, parallel to the root frame.
    root_ranges: Vec<Range<usize>>,
}

impl Builder {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(_) if self.metadata => {}
            Event::Text(text) => match &mut self.code {
                Some((_, code)) => code.push_str(&text.replace('\t', &" ".repeat(TAB_WIDTH))),
                None => self.inline(Inline::Text(text.into_string())),
            },
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { checked, .. }) = self.frames.last_mut() {
                    *checked = Some(done);
                }
            }
            Event::Code(code) => self.inline(Inline::Code(code.into_string())),
            Event::SoftBreak => self.inline(Inline::SoftBreak),
            Event::HardBreak => self.inline(Inline::HardBreak),
            Event::Rule => {
                self.flush_implicit();
                self.block(Block::Rule);
            }
            // Raw HTML, footnotes, math: not shown.
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
            Tag::BlockQuote(kind) => {
                self.flush_implicit();
                if self.open_container() {
                    self.frames
                        .push(Frame::Quote(Vec::new(), kind.map(alert_kind)));
                }
            }
            Tag::MetadataBlock(MetadataBlockKind::YamlStyle | MetadataBlockKind::PlusesStyle) => {
                self.flush_implicit();
                self.metadata = true;
            }
            Tag::Table(aligns) => {
                self.flush_implicit();
                if self.open_container() {
                    self.frames.push(Frame::Table {
                        aligns: aligns.into_iter().map(align).collect(),
                        head: Vec::new(),
                        rows: Vec::new(),
                        row: Vec::new(),
                    });
                }
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(Frame::Table { row, .. }) = self.frames.last_mut() {
                    row.clear();
                }
            }
            Tag::TableCell => {
                if matches!(self.frames.last(), Some(Frame::Table { .. })) {
                    self.spans.push((Span::Cell, Vec::new()));
                }
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
                if self.open_container() {
                    self.frames.push(Frame::List {
                        start: first,
                        items: Vec::new(),
                    });
                }
            }
            Tag::Item => {
                // An item is kept exactly when its list is.
                let real = matches!(self.frames.last(), Some(Frame::List { .. }));
                self.container_real.push(real);
                if real {
                    self.frames.push(Frame::Item {
                        checked: None,
                        blocks: Vec::new(),
                    });
                }
            }
            Tag::Strong => self.open_span(Span::Strong),
            Tag::Emphasis => self.open_span(Span::Emph),
            Tag::Strikethrough => self.open_span(Span::Strike),
            Tag::Link { dest_url, .. } => self.open_span(Span::Link(dest_url.into_string())),
            Tag::Image { dest_url, .. } => self.open_span(Span::Image(dest_url.into_string())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if let Some((_, inlines)) = self.spans.pop() {
                    self.paragraph(inlines);
                }
            }
            TagEnd::Heading(level) => {
                if let Some((_, inlines)) = self.spans.pop() {
                    self.block(Block::Heading(level as u8, inlines));
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush_implicit();
                if self.close_container()
                    && let Some(Frame::Quote(blocks, kind)) = self.frames.pop()
                {
                    self.block(match kind {
                        Some(kind) => Block::Alert { kind, blocks },
                        None => Block::Quote(blocks),
                    });
                }
            }
            TagEnd::MetadataBlock(_) => self.metadata = false,
            TagEnd::Table => {
                if self.close_container()
                    && let Some(Frame::Table {
                        aligns, head, rows, ..
                    }) = self.frames.pop()
                {
                    self.block(Block::Table { aligns, head, rows });
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table { head, row, .. }) = self.frames.last_mut() {
                    *head = std::mem::take(row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table { rows, row, .. }) = self.frames.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::TableCell => {
                if matches!(self.frames.last(), Some(Frame::Table { .. }))
                    && let Some((_, cell)) = self.spans.pop()
                    && let Some(Frame::Table { row, .. }) = self.frames.last_mut()
                {
                    row.push(cell);
                }
            }
            TagEnd::CodeBlock => {
                if let Some((language, text)) = self.code.take() {
                    self.block(Block::Code { language, text });
                }
            }
            TagEnd::List(_) => {
                if self.close_container()
                    && let Some(Frame::List { start, items }) = self.frames.pop()
                {
                    self.block(Block::List { start, items });
                }
            }
            TagEnd::Item => {
                self.flush_implicit();
                if self.close_container()
                    && let Some(Frame::Item { checked, blocks }) = self.frames.pop()
                    && let Some(Frame::List { items, .. }) = self.frames.last_mut()
                {
                    items.push(ListItem { checked, blocks });
                }
            }
            TagEnd::Strong
            | TagEnd::Emphasis
            | TagEnd::Strikethrough
            | TagEnd::Link
            | TagEnd::Image => {
                if !self.span_real.pop().unwrap_or(true) {
                    return;
                }
                if let Some((span, children)) = self.spans.pop() {
                    match span {
                        Span::Strong => self.inline(Inline::Strong(children)),
                        Span::Emph => self.inline(Inline::Emph(children)),
                        Span::Strike => self.inline(Inline::Strike(children)),
                        Span::Link(url) => self.inline(Inline::Link { url, children }),
                        Span::Image(url) => {
                            let alt = plain_text(&children);
                            self.inline(Inline::Image { url, alt });
                        }
                        Span::Base | Span::Cell => {
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

    /// Notes a quote or list tag; true when it gets a frame of its own.
    fn open_container(&mut self) -> bool {
        let real = self.frames.len() < MAX_BLOCK_DEPTH;
        self.container_real.push(real);
        real
    }

    /// Whether the container tag that ends now got a frame.
    fn close_container(&mut self) -> bool {
        self.container_real.pop().unwrap_or(true)
    }

    fn open_span(&mut self, span: Span) {
        let real = self.spans.len() < MAX_INLINE_DEPTH;
        self.span_real.push(real);
        if real {
            self.spans.push((span, Vec::new()));
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
        if let Some((_, inlines)) = self.spans.pop() {
            self.paragraph(inlines);
        }
    }

    /// A paragraph; one holding nothing but an image becomes an image
    /// block, an empty one is dropped.
    fn paragraph(&mut self, inlines: Vec<Inline>) {
        let mut shown = inlines.iter().filter(|i| match i {
            Inline::Text(t) => !t.trim().is_empty(),
            Inline::SoftBreak => false,
            _ => true,
        });
        match (shown.next(), shown.next()) {
            (None, _) => {}
            (Some(Inline::Image { url, alt }), None) => {
                let block = Block::Image {
                    url: url.clone(),
                    alt: alt.clone(),
                };
                self.block(block);
            }
            _ => self.block(Block::Paragraph(inlines)),
        }
    }

    fn block(&mut self, block: Block) {
        match self.frames.last_mut() {
            Some(Frame::Quote(blocks, _) | Frame::Item { blocks, .. }) => blocks.push(block),
            Some(Frame::Root(blocks)) => {
                blocks.push(block);
                self.root_ranges.push(self.root_range.clone());
            }
            _ => {
                self.frames.insert(0, Frame::Root(vec![block]));
                self.root_ranges.push(self.root_range.clone());
            }
        }
    }

    fn finish(mut self) -> (Vec<Block>, Vec<Range<usize>>) {
        self.flush_implicit();
        match self.frames.into_iter().next() {
            Some(Frame::Root(blocks)) => (blocks, self.root_ranges),
            _ => (Vec::new(), Vec::new()),
        }
    }
}

fn alert_kind(kind: BlockQuoteKind) -> AlertKind {
    match kind {
        BlockQuoteKind::Note => AlertKind::Note,
        BlockQuoteKind::Tip => AlertKind::Tip,
        BlockQuoteKind::Important => AlertKind::Important,
        BlockQuoteKind::Warning => AlertKind::Warning,
        BlockQuoteKind::Caution => AlertKind::Caution,
    }
}

fn align(a: Alignment) -> Align {
    match a {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

/// The text of `inlines` without any formatting (an image's alt text).
pub(crate) fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Strong(c) | Inline::Emph(c) | Inline::Strike(c) => out.push_str(&plain_text(c)),
            Inline::Link { children, .. } => out.push_str(&plain_text(children)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
        }
    }
    out
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
pub(crate) fn line_fragment(fragment: &str) -> Point {
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

/// Draws an image: gets the URL, the alt text, the image's index in the
/// rendered blocks (document order) and the app. Supplied by the preview,
/// which loads files; without one, images are drawn as placeholders.
pub type ImageLookup = Rc<dyn Fn(&str, &str, usize, &App) -> AnyElement>;

/// Sizes and spacing a markdown render uses: hover popovers and the
/// preview share the renderer and differ only here.
#[derive(Clone)]
pub struct MarkdownStyle {
    /// Body text size in px.
    pub text_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Space between root blocks in px ([`render_markdown_with`] only; a
    /// list of blocks lays them out itself).
    pub block_gap: f32,
    /// Font size in px of h1 to h6.
    pub heading_sizes: [f32; 6],
    /// A rule under h1 and h2.
    pub heading_rules: bool,
    /// Space above and below a heading in px.
    pub heading_margin: (f32, f32),
    /// Horizontal and vertical padding of a code block in px.
    pub code_padding: (f32, f32),
    /// Long code lines scroll sideways instead of wrapping.
    pub code_scroll_x: bool,
    /// Width in px of the bar left of quotes and alerts.
    pub quote_bar: f32,
    /// The background of code blocks and table headers.
    pub code_background: CodeBackground,
    /// Draws images; `None` draws placeholders.
    pub images: Option<ImageLookup>,
    /// The font of text that is not code.
    pub font_family: SharedString,
    /// The font of code (inline and blocks): the window's
    /// `editor.fontFamily`, the bundled editor font by default.
    pub code_font_family: SharedString,
}

impl MarkdownStyle {
    /// Hover popovers: compact text, no rules. Code blocks scroll sideways,
    /// as they always did there.
    pub fn hover() -> Self {
        Self {
            text_size: UI_FONT_SIZE_SMALL,
            line_height: std::f32::consts::GOLDEN_RATIO, // gpui's default
            block_gap: 8.,
            heading_sizes: [18., 16., 15., 14., 14., 14.],
            heading_rules: false,
            heading_margin: (0., 0.),
            code_padding: (8., 6.),
            code_scroll_x: true,
            quote_bar: 2.,
            code_background: CodeBackground::Editor,
            images: None,
            font_family: UI_FONT_FAMILY.into(),
            code_font_family: EDITOR_FONT_FAMILY.into(),
        }
    }

    /// The preview: headings scale with `font_size`.
    pub fn preview(font_size: f32, line_height: f32, images: Option<ImageLookup>) -> Self {
        Self {
            text_size: font_size,
            line_height,
            block_gap: 16.,
            heading_sizes: [2.0, 1.5, 1.25, 1., 1., 1.].map(|k| k * font_size),
            heading_rules: true,
            heading_margin: (24., 12.),
            code_padding: (12., 12.),
            code_scroll_x: true,
            quote_bar: 3.,
            code_background: CodeBackground::Elevated,
            images,
            font_family: UI_FONT_FAMILY.into(),
            code_font_family: EDITOR_FONT_FAMILY.into(),
        }
    }
}

/// Which theme color backs code blocks and table headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeBackground {
    /// `editor_background`: what hover popovers use.
    Editor,
    /// `elevated_surface_background`: contrasts with the editor page.
    Elevated,
}

fn code_background(style: &MarkdownStyle, ui: &lucerna_theme::UiColors) -> gpui::Rgba {
    color(match style.code_background {
        CodeBackground::Editor => ui.editor_background,
        CodeBackground::Elevated => ui.elevated_surface_background,
    })
}

/// A bordered stand-in for image `n` (document order) when it is not
/// drawn: an icon, the alt text and an optional note (why there is no
/// image).
pub(crate) fn image_placeholder(alt: &str, note: Option<&str>, n: usize, cx: &App) -> AnyElement {
    image_placeholder_builder(alt, note, n, cx)()
}

/// Builds [`image_placeholder`] later, without the app (an image that
/// fails to load draws it in its place).
pub(crate) fn image_placeholder_builder(
    alt: &str,
    note: Option<&str>,
    n: usize,
    cx: &App,
) -> impl Fn() -> AnyElement + 'static {
    let ui = &cx.global::<AppState>().theme.ui;
    let (border, text, icon) = (
        color(ui.border_variant),
        color(ui.text_muted),
        color(ui.icon),
    );
    let label = if alt.is_empty() { "image" } else { alt }.to_owned();
    let note = note.map(str::to_owned);
    move || {
        div()
            .debug_selector(move || format!("md-image-alt-{n}"))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .border_1()
            .border_color(border)
            .text_color(text)
            .child(
                svg()
                    .data(icons::IMAGE)
                    .size(px(14.))
                    .flex_none()
                    .text_color(icon),
            )
            .child(label.clone())
            .when_some(note.clone(), |d, note| d.child(note))
            .into_any_element()
    }
}

/// Renders `blocks`; clicking a web link opens the browser. The default
/// handler ignores file links: use [`render_markdown_with`] for those.
pub fn render_markdown(blocks: &MarkdownBlocks, cx: &App) -> AnyElement {
    render_markdown_with(
        blocks,
        Rc::new(|url, _, cx| {
            if let Some(LinkTarget::Web(url)) = link_target(url) {
                cx.open_url(&url);
            }
        }),
        EDITOR_FONT_FAMILY.into(),
        cx,
    )
}

/// Renders `blocks` as a hover popover, its code in `code_font_family`;
/// `on_link` gets the URL of a clicked link.
pub fn render_markdown_with(
    blocks: &MarkdownBlocks,
    on_link: LinkHandler,
    code_font_family: SharedString,
    cx: &App,
) -> AnyElement {
    let mut style = MarkdownStyle::hover();
    style.code_font_family = code_font_family;
    div()
        .flex()
        .flex_col()
        .gap(px(style.block_gap))
        .min_w_0()
        .children(
            (0..blocks.blocks.len())
                .map(|root| render_block(blocks, root, &style, on_link.clone(), cx)),
        )
        .into_any_element()
}

/// Renders root block `root` of `blocks` (an empty element when there is
/// no such block). Code blocks and images are numbered as in the whole
/// document.
pub fn render_block(
    blocks: &MarkdownBlocks,
    root: usize,
    style: &MarkdownStyle,
    on_link: LinkHandler,
    cx: &App,
) -> AnyElement {
    let Some(block) = blocks.blocks.get(root) else {
        return div().into_any_element();
    };
    let mut renderer = Renderer {
        blocks,
        cx,
        style,
        on_link,
        root,
        texts: 0,
        codes: blocks.code_index_start(root),
        images: blocks.image_index_start(root),
        tasks: 0,
        tables: 0,
        links: 0,
    };
    let element = renderer.block(block, style.text_size, false);
    div()
        .min_w_0()
        .font_family(style.font_family.clone())
        .text_size(px(style.text_size))
        .line_height(relative(style.line_height))
        .text_color(color(cx.global::<AppState>().theme.ui.text))
        .child(element)
        .into_any_element()
}

struct Renderer<'a> {
    blocks: &'a MarkdownBlocks,
    cx: &'a App,
    style: &'a MarkdownStyle,
    on_link: LinkHandler,
    /// The root block being drawn: keeps element ids unique per block.
    root: usize,
    /// Ids for the clickable text elements.
    texts: usize,
    /// Code blocks drawn so far: the next one's index.
    codes: usize,
    /// Images drawn so far in the document: the next one's index.
    images: usize,
    /// Task list items drawn so far in this block.
    tasks: usize,
    /// Tables drawn so far in this block (ids).
    tables: usize,
    /// Clickable images drawn so far in this block (ids).
    links: usize,
}

/// The font and color a run of inlines is drawn with.
#[derive(Clone, Copy)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
    muted: bool,
    strike: bool,
}

impl Renderer<'_> {
    /// An element id unique to this block.
    fn id(&self, kind: &'static str, n: usize) -> ElementId {
        ElementId::NamedInteger(format!("{kind}-{}", self.root).into(), n as u64)
    }

    fn blocks(&mut self, blocks: &[Block], size: f32, muted: bool) -> Vec<AnyElement> {
        blocks
            .iter()
            .map(|block| self.block(block, size, muted))
            .collect()
    }

    fn block(&mut self, block: &Block, size: f32, muted: bool) -> AnyElement {
        let ui = &self.cx.global::<AppState>().theme.ui;
        let bar = self.style.quote_bar;
        match block {
            Block::Paragraph(inlines) => self.text(inlines, size, false, muted),
            Block::Heading(level, inlines) => self.heading(*level, inlines, muted),
            Block::Code { .. } => self.code(),
            Block::List { start, items } => self.list(*start, items, size, muted),
            Block::Alert { kind, blocks } => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pl(px(8.))
                .border_l(px(bar))
                .border_color(color(ui.border))
                .child(div().font_weight(FontWeight::BOLD).child(kind.title()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .text_color(color(ui.text_muted))
                        .children(self.blocks(blocks, size, true)),
                )
                .into_any_element(),
            Block::Table { aligns, head, rows } => self.table(aligns, head, rows, size),
            Block::Image { url, alt } => self.image(url, alt),
            Block::Quote(inner) => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .pl(px(8.))
                .border_l(px(bar))
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

    fn heading(&mut self, level: u8, inlines: &[Inline], muted: bool) -> AnyElement {
        let ui = &self.cx.global::<AppState>().theme.ui;
        let size = self.style.heading_sizes[(level.clamp(1, 6) - 1) as usize];
        let (above, below) = self.style.heading_margin;
        let rule = self.style.heading_rules && level <= 2;
        div()
            .mt(px(above))
            .mb(px(below))
            .when(rule, |d| {
                d.pb(px(4.))
                    .border_b_1()
                    .border_color(color(ui.border_variant))
            })
            .child(self.text(inlines, size, true, muted))
            .into_any_element()
    }

    fn list(
        &mut self,
        start: Option<u64>,
        items: &[ListItem],
        size: f32,
        muted: bool,
    ) -> AnyElement {
        let mut rows = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let marker = match (start, item.checked) {
                (_, Some(checked)) => {
                    let n = self.tasks;
                    self.tasks += 1;
                    div()
                        .debug_selector(move || format!("md-task-{n}"))
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(px(size * self.style.line_height))
                        .child(checkbox(checked, self.cx))
                        .into_any_element()
                }
                (Some(first), None) => div()
                    .flex_none()
                    .child(format!("{}.", first + i as u64))
                    .into_any_element(),
                (None, None) => div().flex_none().child("•").into_any_element(),
            };
            let body = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(self.blocks(&item.blocks, size, muted));
            rows.push(div().flex().gap(px(6.)).child(marker).child(body));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(rows)
            .into_any_element()
    }

    /// A bordered table: columns are as wide as their widest cell, and
    /// the table scrolls sideways on its own when wider than the frame.
    fn table(
        &mut self,
        aligns: &[Align],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        size: f32,
    ) -> AnyElement {
        let ui = &self.cx.global::<AppState>().theme.ui;
        let columns = aligns.len().max(head.len());
        let id = self.tables;
        self.tables += 1;
        // One grid, cells added row by row: a row is as tall as its
        // tallest cell, a column as wide as its widest.
        let last_row = rows.len();
        let last_column = columns.saturating_sub(1);
        let mut cells = Vec::with_capacity(columns * (rows.len() + 1));
        for (r, row) in std::iter::once(head)
            .chain(rows.iter().map(Vec::as_slice))
            .enumerate()
        {
            for c in 0..columns {
                let inlines: &[Inline] = row.get(c).map(Vec::as_slice).unwrap_or(&[]);
                let cell = self.text(inlines, size, r == 0, false);
                let align = aligns.get(c).copied().unwrap_or(Align::None);
                cells.push(
                    div()
                        .debug_selector(move || format!("md-table-cell-{r}-{c}"))
                        .px(px(10.))
                        .py(px(6.))
                        .border_color(color(ui.border_variant))
                        .when(r == 0, |d| d.bg(code_background(self.style, ui)))
                        .when(r < last_row, |d| d.border_b_1())
                        .when(c < last_column, |d| d.border_r_1())
                        .map(|d| match align {
                            Align::Center => d.text_center(),
                            Align::Right => d.text_right(),
                            Align::None | Align::Left => d.text_left(),
                        })
                        .child(cell)
                        .into_any_element(),
                );
            }
        }
        let table = div()
            .grid()
            .grid_cols_max_content(columns as u16)
            .w_full()
            .border_1()
            .border_color(color(ui.border_variant))
            .rounded(px(4.))
            .overflow_hidden()
            .children(cells);
        div()
            .id(self.id("md-table-scroll", id))
            .flex()
            .w_full()
            .overflow_x_scroll()
            .child(table)
            .into_any_element()
    }

    /// The next image in the document.
    fn image(&mut self, url: &str, alt: &str) -> AnyElement {
        let n = self.images;
        self.images += 1;
        match &self.style.images {
            Some(lookup) => lookup(url, alt, n, self.cx),
            None => image_placeholder(alt, None, n, self.cx),
        }
    }

    /// One text element for a paragraph, heading or table cell; inline
    /// images split it into a wrapping row of text and images.
    fn text(&mut self, inlines: &[Inline], size: f32, bold: bool, muted: bool) -> AnyElement {
        let mut flat = Flat::default();
        let style = InlineStyle {
            bold,
            italic: false,
            code: false,
            link: false,
            muted,
            strike: false,
        };
        self.flatten(inlines, style, None, &mut flat);
        flat.flush();
        let mut segs = flat.segs;
        match segs.len() {
            0 => div().into_any_element(),
            1 if matches!(segs[0], Seg::Text(_)) => {
                let Some(Seg::Text(chunk)) = segs.pop() else {
                    unreachable!()
                };
                self.chunk(chunk, size)
            }
            _ => {
                let children: Vec<AnyElement> = segs
                    .into_iter()
                    .map(|seg| match seg {
                        Seg::Text(chunk) => self.chunk(chunk, size),
                        Seg::Image { url, alt, link } => {
                            let image = self.image(&url, &alt);
                            self.linked(image, link)
                        }
                    })
                    .collect();
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(4.))
                    .children(children)
                    .into_any_element()
            }
        }
    }

    /// Makes `element` clickable when it sits inside a link.
    fn linked(&mut self, element: AnyElement, link: Option<String>) -> AnyElement {
        let Some(url) = link else { return element };
        self.links += 1;
        let on_link = self.on_link.clone();
        let tip = url.clone();
        div()
            .id(self.id("md-image-link", self.links))
            .cursor_pointer()
            .tooltip(tooltip(tip))
            .on_click(move |_, window, cx| on_link(&url, window, cx))
            .child(element)
            .into_any_element()
    }

    /// A run of styled text, with its links clickable.
    fn chunk(&mut self, chunk: Chunk, size: f32) -> AnyElement {
        let styled = StyledText::new(chunk.text).with_runs(chunk.runs);
        let el = div().text_size(px(size));
        if chunk.links.is_empty() {
            return el.child(styled).into_any_element();
        }
        self.texts += 1;
        let (ranges, urls): (Vec<_>, Vec<_>) = chunk.links.into_iter().unzip();
        let urls = Rc::new(urls);
        let on_link = self.on_link.clone();
        let (root, n) = (self.root, self.texts);
        // The URL of the link under the pointer, as a tooltip.
        let tips = (ranges.clone(), urls.clone());
        let clicked = urls.clone();
        el.debug_selector(move || format!("md-text-{root}-{n}"))
            .child(
                InteractiveText::new(self.id("md-text", n), styled)
                    .on_click(ranges, move |ix, window, cx| {
                        on_link(&clicked[ix], window, cx)
                    })
                    .tooltip(move |index, window, cx| {
                        let ix = tips.0.iter().position(|r| r.contains(&index))?;
                        Some(tooltip(tips.1[ix].clone())(window, cx))
                    }),
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
                self.style.code_font_family.clone()
            } else {
                self.style.font_family.clone()
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
            let cur = &mut out.cur;
            let start = cur.text.len();
            cur.text.push_str(text);
            cur.runs.push(TextRun {
                len: text.len(),
                font: f,
                color: color(text_color).into(),
                background_color: style.code.then(|| color(ui.editor_background).into()),
                underline: style.link.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                strikethrough: style.strike.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
            });
            if let Some(url) = link {
                match cur.links.last_mut() {
                    Some((range, last)) if range.end == start && last == url => {
                        range.end = cur.text.len()
                    }
                    _ => cur.links.push((start..cur.text.len(), url.to_owned())),
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
                Inline::Strike(children) => self.flatten(
                    children,
                    InlineStyle {
                        strike: true,
                        ..style
                    },
                    link,
                    out,
                ),
                Inline::Image { url, alt } => {
                    out.flush();
                    out.segs.push(Seg::Image {
                        url: url.clone(),
                        alt: alt.clone(),
                        link: link.map(str::to_owned),
                    });
                }
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

    /// The next code block: code font, code background, sideways scroll
    /// or wrapping as the style says.
    fn code(&mut self) -> AnyElement {
        let index = self.codes;
        self.codes += 1;
        let ui = &self.cx.global::<AppState>().theme.ui;
        let (text, runs) = self
            .blocks
            .code_runs(index, &self.style.code_font_family, self.cx)
            .unwrap_or_default();
        let (px_, py_) = self.style.code_padding;
        let scroll = self.style.code_scroll_x;
        let code = div()
            .id(("md-code-scroll", index))
            .w_full()
            .when(scroll, |d| d.overflow_x_scroll())
            .rounded(px(GRID))
            .bg(code_background(self.style, ui))
            .child(
                div()
                    .px(px(px_))
                    .py(px(py_))
                    .when(scroll, |d| d.whitespace_nowrap())
                    .text_size(px(self.style.text_size))
                    .child(StyledText::new(text).with_runs(runs.to_vec())),
            );
        div()
            .debug_selector(move || format!("md-code-{index}"))
            .w_full()
            .child(code)
            .into_any_element()
    }
}

/// A run of text with one style per `runs` entry.
#[derive(Default)]
struct Chunk {
    text: String,
    runs: Vec<TextRun>,
    /// Byte ranges of `text` and their URLs.
    links: Vec<(Range<usize>, String)>,
}

enum Seg {
    Text(Chunk),
    Image {
        url: String,
        alt: String,
        /// The link the image sits in.
        link: Option<String>,
    },
}

/// Inlines flattened: text runs and the images between them.
#[derive(Default)]
struct Flat {
    cur: Chunk,
    segs: Vec<Seg>,
}

impl Flat {
    /// Closes the text collected so far.
    fn flush(&mut self) {
        if !self.cur.text.is_empty() {
            self.segs.push(Seg::Text(std::mem::take(&mut self.cur)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Point;
    use std::path::PathBuf;

    fn item(blocks: Vec<Block>) -> ListItem {
        ListItem {
            checked: None,
            blocks,
        }
    }

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
                start: None,
                items: vec![
                    item(vec![
                        Block::Paragraph(vec![text("a")]),
                        Block::List {
                            start: None,
                            items: vec![
                                item(vec![Block::Paragraph(vec![text("b")])]),
                                item(vec![Block::Paragraph(vec![text("c")])]),
                            ],
                        },
                    ]),
                    item(vec![Block::Paragraph(vec![text("d")])]),
                ],
            }]
        );
        let ordered = parse("1. x\n2. y\n");
        assert!(
            matches!(&ordered[..], [Block::List { start: Some(1), items }] if items.len() == 2)
        );
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

    fn depth(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::Quote(inner) => 1 + depth(inner),
                Block::List { items, .. } => {
                    1 + items.iter().map(|i| depth(&i.blocks)).max().unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    fn contains_text(blocks: &[Block], needle: &str) -> bool {
        blocks.iter().any(|b| match b {
            Block::Paragraph(inlines) => inlines.iter().any(|i| i == &text(needle)),
            Block::Quote(inner) => contains_text(inner, needle),
            Block::List { items, .. } => items.iter().any(|i| contains_text(&i.blocks, needle)),
            _ => false,
        })
    }

    #[test]
    fn deep_nesting_is_flattened() {
        let quotes = parse(&format!("{}x", "> ".repeat(49_000)));
        assert!(depth(&quotes) <= MAX_BLOCK_DEPTH, "{}", depth(&quotes));
        assert!(contains_text(&quotes, "x"));

        let lists = parse(&format!("{}x", "- ".repeat(49_000)));
        assert!(depth(&lists) <= MAX_BLOCK_DEPTH, "{}", depth(&lists));
        assert!(contains_text(&lists, "x"));

        let indented: String = (0..200)
            .map(|i| format!("{}- x{i}\n", "  ".repeat(i)))
            .collect();
        let nested = parse(&indented);
        assert!(depth(&nested) <= MAX_BLOCK_DEPTH);
        assert!(contains_text(&nested, "x199"));

        let emphasis = parse(&format!("{}x", "*_".repeat(20_000)));
        drop(emphasis);
        let links = parse(&format!("{}x{}", "[".repeat(20_000), "](u)".repeat(20_000)));
        drop(links);
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
