//! Where the preview's images and links point (spec §5.6): image URLs
//! resolved against the previewed file and confined to the workspace (or
//! the file's folder), links sorted by what a click opens, and the
//! GitHub-style slugs `#anchor` links name headings by.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use lucerna_text::Point;

use crate::markdown::{Block, MarkdownBlocks, line_fragment, plain_text, visit_image_urls};

/// Image files larger than this (in bytes) are not loaded.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// What an image URL of the previewed file resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSrc {
    /// A file to load (canonical path).
    Local(PathBuf),
    /// Not loaded: `"remote"`, `"missing"`, `"outside"` or `"too-large"`.
    Blocked(&'static str),
}

/// Where the image URLs of one markdown file are resolved from, and the
/// folders its images may come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageContext {
    /// The file's folder: relative URLs start here.
    pub base_dir: PathBuf,
    /// The workspace root (the file's folder in single-file mode): URLs
    /// starting with `/` start here.
    pub root: PathBuf,
    /// The folders images must lie under, as given (they are canonicalized
    /// when compared).
    pub allowed: Vec<PathBuf>,
}

impl ImageContext {
    /// The context of `file` (absolute) in the workspace at `root`: images
    /// from the root when the file is inside it, else only from the file's
    /// own folder (as VS Code's `localResourceRoots`). Does not touch the
    /// disk.
    pub fn for_file(file: &Path, root: &Path) -> Self {
        let base_dir = file.parent().unwrap_or(root).to_path_buf();
        let allowed = if file.starts_with(root) {
            root.to_path_buf()
        } else {
            base_dir.clone()
        };
        ImageContext {
            base_dir,
            root: root.to_path_buf(),
            allowed: vec![allowed],
        }
    }
}

/// Resolves image `url` of the file `ctx` describes: remote and `data:`
/// URLs are not loaded; `/x` starts at the root, other paths at the file's
/// folder, `%xx` decoded and `..` folded; the file must exist, lie under
/// one of `ctx.allowed` once symlinks are followed, and be at most
/// [`MAX_IMAGE_BYTES`]. Touches the disk: runs on the background parse.
pub fn resolve_image(url: &str, ctx: &ImageContext) -> ImageSrc {
    let path = match scheme(url) {
        Some(s) if s.eq_ignore_ascii_case("file") => {
            match url::Url::parse(url)
                .ok()
                .and_then(|u| u.to_file_path().ok())
            {
                Some(path) => path,
                None => return ImageSrc::Blocked("missing"),
            }
        }
        Some(_) => return ImageSrc::Blocked("remote"),
        None => {
            let (path, _) = split_fragment(url);
            if path.is_empty() {
                return ImageSrc::Blocked("missing");
            }
            local_path(&percent_decode(path), &ctx.base_dir, &ctx.root)
        }
    };
    let Ok(real) = path.canonicalize() else {
        return ImageSrc::Blocked("missing");
    };
    let inside = ctx
        .allowed
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .any(|dir| real.starts_with(dir));
    if !inside {
        return ImageSrc::Blocked("outside");
    }
    match real.metadata() {
        Ok(meta) if !meta.is_file() => ImageSrc::Blocked("missing"),
        Ok(meta) if meta.len() > MAX_IMAGE_BYTES => ImageSrc::Blocked("too-large"),
        Ok(_) => ImageSrc::Local(real),
        Err(_) => ImageSrc::Blocked("missing"),
    }
}

/// Resolves every image URL of `blocks` (see [`resolve_image`]).
pub fn resolve_images(blocks: &MarkdownBlocks, ctx: &ImageContext) -> HashMap<String, ImageSrc> {
    let mut images = HashMap::new();
    visit_image_urls(blocks.blocks(), &mut |url| {
        if !images.contains_key(url) {
            images.insert(url.to_owned(), resolve_image(url, ctx));
        }
    });
    images
}

/// What a click on a link of the preview opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// A web or `mailto:` URL, for the browser or the default app.
    Web(String),
    /// A heading of the previewed file, by slug.
    Anchor(String),
    /// Another markdown file, and the heading to scroll to.
    Markdown { path: PathBuf, slug: Option<String> },
    /// A file's source at a place (`#L10`, `#L10,5`).
    Source { path: PathBuf, point: Point },
    /// Any other file.
    File(PathBuf),
    /// Not a link Lucerna follows.
    None,
}

/// Sorts link `url` of a markdown file in `base_dir` (workspace `root`):
/// `http(s):` and `mailto:` go to the browser, `#slug` scrolls the
/// preview, and relative, `/`-rooted or `file://` paths open markdown
/// files in the preview and other files as the Explorer would; `#L10`
/// opens the source at that line.
pub fn resolve_link(url: &str, base_dir: &Path, root: &Path) -> LinkAction {
    let (path, fragment) = match scheme(url) {
        Some(s)
            if ["http", "https", "mailto"]
                .iter()
                .any(|w| s.eq_ignore_ascii_case(w)) =>
        {
            return LinkAction::Web(url.to_owned());
        }
        Some(s) if s.eq_ignore_ascii_case("file") => {
            let Ok(parsed) = url::Url::parse(url) else {
                return LinkAction::None;
            };
            let Ok(path) = parsed.to_file_path() else {
                return LinkAction::None;
            };
            (path, parsed.fragment().map(percent_decode))
        }
        Some(_) => return LinkAction::None,
        None => {
            let (path, fragment) = split_fragment(url);
            let fragment = fragment.map(percent_decode);
            if path.is_empty() {
                return match fragment {
                    Some(slug) if !slug.is_empty() => LinkAction::Anchor(slug),
                    _ => LinkAction::None,
                };
            }
            (local_path(&percent_decode(path), base_dir, root), fragment)
        }
    };
    let fragment = fragment.filter(|f| !f.is_empty());
    if let Some(line) = fragment.as_deref().filter(|f| is_line_fragment(f)) {
        return LinkAction::Source {
            path,
            point: line_fragment(line),
        };
    }
    if is_markdown_path(&path) {
        LinkAction::Markdown {
            path,
            slug: fragment,
        }
    } else {
        LinkAction::File(path)
    }
}

/// The slug of each root heading of `blocks`, GitHub style, with its root
/// block index: lowercase, characters other than letters, digits, `_`,
/// `-` and whitespace dropped, each whitespace turned into `-`; a slug
/// seen before gets `-1`, `-2`, ….
pub fn heading_slugs(blocks: &MarkdownBlocks) -> Vec<(String, usize)> {
    let mut seen = HashSet::new();
    let mut slugs = Vec::new();
    for (ix, block) in blocks.blocks().iter().enumerate() {
        let Block::Heading(_, inlines) = block else {
            continue;
        };
        let base = slug(&plain_text(inlines));
        let mut candidate = base.clone();
        let mut n = 0;
        while !seen.insert(candidate.clone()) {
            n += 1;
            candidate = format!("{base}-{n}");
        }
        slugs.push((candidate, ix));
    }
    slugs
}

/// `text` as a GitHub heading slug (before de-duplication).
fn slug(text: &str) -> String {
    text.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// The scheme of `url` (`https` in `https://…`); `None` for a path. A
/// one-letter scheme is a Windows drive (`C:\…`), not a scheme.
fn scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (scheme.len() > 1
        && first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(scheme)
}

/// `url` without its query, and its `#` fragment.
fn split_fragment(url: &str) -> (&str, Option<&str>) {
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    };
    let path = rest.split_once('?').map_or(rest, |(path, _)| path);
    (path, fragment)
}

/// `path` from a link: `/x` under `root`, else under `base_dir`, with `.`
/// and `..` folded without touching the disk.
fn local_path(path: &str, base_dir: &Path, root: &Path) -> PathBuf {
    let joined = match path.strip_prefix('/') {
        Some(rest) => root.join(rest),
        None => base_dir.join(path),
    };
    normalize(&joined)
}

/// Folds `.` and `..` in `path` lexically; `..` never climbs above the
/// root.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Decodes `%xx` escapes; an invalid escape stays as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).and_then(|&b| hex(b)),
                bytes.get(i + 2).and_then(|&b| hex(b)),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `L10`, `L10,5`, …: a line of the file, not a heading.
fn is_line_fragment(fragment: &str) -> bool {
    fragment
        .strip_prefix('L')
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

/// Whether `path` names a markdown file by its extension.
fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}
