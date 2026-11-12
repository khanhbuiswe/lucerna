//! The bundled Lucide icons (ISC, see `assets/icons/LICENSE-lucide`). Which
//! icon a file gets is the icon theme's business (`lucerna_theme`).
//!
//! The SVGs are upstream files, unmodified. They draw with
//! `stroke="currentColor"`, which resvg resolves to opaque black when no
//! `color` is set; GPUI uses the rendered alpha as a mask and fills it with
//! the element's `text_color`, so the stroke takes the theme's icon color.

macro_rules! lucide {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/",
            $name,
            ".svg"
        ))
    };
}

pub const FOLDER: &[u8] = lucide!("folder");
pub const FOLDER_OPEN: &[u8] = lucide!("folder-open");
pub const FILE: &[u8] = lucide!("file");
pub const FILE_CODE: &[u8] = lucide!("file-code");
pub const BRACES: &[u8] = lucide!("braces");
pub const BOOK_OPEN: &[u8] = lucide!("book-open");
pub const SETTINGS: &[u8] = lucide!("settings");
pub const LOCK: &[u8] = lucide!("lock");
pub const GIT_BRANCH: &[u8] = lucide!("git-branch");
pub const SCALE: &[u8] = lucide!("scale");
pub const TERMINAL: &[u8] = lucide!("terminal");
pub const IMAGE: &[u8] = lucide!("image");
pub const SEARCH: &[u8] = lucide!("search");
pub const FILES: &[u8] = lucide!("files");
pub const X: &[u8] = lucide!("x");
pub const CHEVRON_RIGHT: &[u8] = lucide!("chevron-right");
pub const CHEVRON_DOWN: &[u8] = lucide!("chevron-down");
pub const CHEVRON_UP: &[u8] = lucide!("chevron-up");
pub const BLOCKS: &[u8] = lucide!("blocks");
pub const LOADER_CIRCLE: &[u8] = lucide!("loader-circle");
pub const LIST_FILTER: &[u8] = lucide!("list-filter");
pub const REFRESH_CW: &[u8] = lucide!("refresh-cw");
pub const CHEVRONS_DOWN_UP: &[u8] = lucide!("chevrons-down-up");
pub const CHEVRON_LEFT: &[u8] = lucide!("chevron-left");
pub const CHEVRONS_UP_DOWN: &[u8] = lucide!("chevrons-up-down");
pub const ARROW_UP_RIGHT: &[u8] = lucide!("arrow-up-right");
pub const CASE_SENSITIVE: &[u8] = lucide!("case-sensitive");
pub const WHOLE_WORD: &[u8] = lucide!("whole-word");
pub const REGEX: &[u8] = lucide!("regex");
pub const TRIANGLE_ALERT: &[u8] = lucide!("triangle-alert");
pub const CIRCLE_X: &[u8] = lucide!("circle-x");
pub const INFO: &[u8] = lucide!("info");
pub const CHECK: &[u8] = lucide!("check");
pub const ZAP: &[u8] = lucide!("zap");
pub const SQUARE_FUNCTION: &[u8] = lucide!("square-function");
pub const BOX: &[u8] = lucide!("box");
pub const COMPONENT: &[u8] = lucide!("component");
pub const LIST: &[u8] = lucide!("list");
pub const BRACKETS: &[u8] = lucide!("brackets");
pub const VARIABLE: &[u8] = lucide!("variable");
pub const HASH: &[u8] = lucide!("hash");
pub const PACKAGE: &[u8] = lucide!("package");
pub const TYPE: &[u8] = lucide!("type");
pub const CIRCLE: &[u8] = lucide!("circle");
pub const HISTORY: &[u8] = lucide!("history");

/// The Lucerna mark (`assets/brand/mark.svg` at the repository root), in
/// its own colors: draw it as an image, not as a single-color `svg`.
pub const LUCERNA_MARK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/brand/mark.svg"
));

/// Every bundled icon with its upstream name.
pub const ALL: &[(&str, &[u8])] = &[
    ("folder", FOLDER),
    ("folder-open", FOLDER_OPEN),
    ("file", FILE),
    ("file-code", FILE_CODE),
    ("braces", BRACES),
    ("book-open", BOOK_OPEN),
    ("settings", SETTINGS),
    ("lock", LOCK),
    ("git-branch", GIT_BRANCH),
    ("scale", SCALE),
    ("terminal", TERMINAL),
    ("image", IMAGE),
    ("search", SEARCH),
    ("files", FILES),
    ("x", X),
    ("chevron-right", CHEVRON_RIGHT),
    ("chevron-down", CHEVRON_DOWN),
    ("chevron-up", CHEVRON_UP),
    ("blocks", BLOCKS),
    ("loader-circle", LOADER_CIRCLE),
    ("list-filter", LIST_FILTER),
    ("refresh-cw", REFRESH_CW),
    ("chevrons-down-up", CHEVRONS_DOWN_UP),
    ("chevron-left", CHEVRON_LEFT),
    ("chevrons-up-down", CHEVRONS_UP_DOWN),
    ("arrow-up-right", ARROW_UP_RIGHT),
    ("case-sensitive", CASE_SENSITIVE),
    ("whole-word", WHOLE_WORD),
    ("regex", REGEX),
    ("triangle-alert", TRIANGLE_ALERT),
    ("circle-x", CIRCLE_X),
    ("info", INFO),
    ("check", CHECK),
    ("zap", ZAP),
    ("square-function", SQUARE_FUNCTION),
    ("box", BOX),
    ("component", COMPONENT),
    ("list", LIST),
    ("brackets", BRACKETS),
    ("variable", VARIABLE),
    ("hash", HASH),
    ("package", PACKAGE),
    ("type", TYPE),
    ("circle", CIRCLE),
    ("history", HISTORY),
];

/// The bundled SVG for a Lucide icon name.
pub fn lucide_svg(name: &str) -> Option<&'static [u8]> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, svg)| *svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucide_svg_resolves_every_bundled_name() {
        for (name, svg) in ALL {
            assert!(lucide_svg(name) == Some(*svg), "{name}");
        }
        assert!(lucide_svg("nope").is_none());
    }

    #[test]
    fn vendored_svgs_are_upstream_lucide() {
        for (name, svg) in ALL {
            let text = std::str::from_utf8(svg).unwrap();
            assert!(text.contains(r#"viewBox="0 0 24 24""#), "{name}");
            assert!(text.contains(r#"stroke="currentColor""#), "{name}");
        }
    }
}
