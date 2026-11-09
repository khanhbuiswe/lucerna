//! The bundled Lucide icons (ISC, see `assets/icons/LICENSE-lucide`) and
//! the file-type icon of each Explorer row.
//!
//! The SVGs are upstream files, unmodified. They draw with
//! `stroke="currentColor"`, which resvg resolves to opaque black when no
//! `color` is set; GPUI uses the rendered alpha as a mask and fills it with
//! the element's `text_color`, so the stroke takes the theme's icon color.

use std::path::Path;

use lucerna_theme::{IconSource, IconTheme, lucide_icon_theme};

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
];

/// The bundled SVG for a Lucide icon name.
pub fn lucide_svg(name: &str) -> Option<&'static [u8]> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, svg)| *svg)
}

/// The icon for a tree entry, through the built-in Lucide theme. Temporary:
/// the Explorer moves to the active icon theme later.
pub fn icon_for_path(path: &Path, is_dir: bool, expanded: bool) -> &'static [u8] {
    static LUCIDE: std::sync::OnceLock<IconTheme> = std::sync::OnceLock::new();
    let theme = LUCIDE.get_or_init(lucide_icon_theme);
    let icon = if is_dir {
        let name = path.file_name().map(|n| n.to_string_lossy());
        theme.directory_icon(name.as_deref().unwrap_or(""), expanded)
    } else {
        theme.file_icon(path)
    };
    match icon {
        Some(IconSource::Builtin(name)) => lucide_svg(name).unwrap_or(FILE),
        _ => FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> &'static [u8] {
        icon_for_path(Path::new(name), false, false)
    }

    #[test]
    fn directories_use_folder_icons() {
        assert_eq!(icon_for_path(Path::new("src"), true, false), FOLDER);
        assert_eq!(icon_for_path(Path::new("src"), true, true), FOLDER_OPEN);
        // A directory named like a file is still a folder.
        assert_eq!(icon_for_path(Path::new("x.json"), true, false), FOLDER);
    }

    #[test]
    fn files_map_by_name_then_extension() {
        let cases: &[(&str, &[u8])] = &[
            ("Cargo.lock", LOCK),
            ("pnpm-lock.lock", LOCK),
            ("LICENSE", SCALE),
            ("LICENSE-MIT", SCALE),
            ("LICENSE.md", SCALE),
            ("COPYING", SCALE),
            (".gitignore", GIT_BRANCH),
            (".gitattributes", GIT_BRANCH),
            ("sub/.gitmodules", GIT_BRANCH),
            ("package.json", BRACES),
            ("tsconfig.jsonc", BRACES),
            ("README.md", BOOK_OPEN),
            ("notes.markdown", BOOK_OPEN),
            ("Cargo.toml", SETTINGS),
            ("ci.yaml", SETTINGS),
            ("ci.YML", SETTINGS),
            ("build.sh", TERMINAL),
            ("x.bash", TERMINAL),
            ("x.zsh", TERMINAL),
            ("logo.png", IMAGE),
            ("a.jpg", IMAGE),
            ("a.jpeg", IMAGE),
            ("a.gif", IMAGE),
            ("icon.svg", IMAGE),
            ("a.webp", IMAGE),
            ("favicon.ico", IMAGE),
            ("Makefile", FILE),
            ("notes.txt", FILE),
            ("", FILE),
        ];
        for (name, icon) in cases {
            assert!(file(name) == *icon, "{name}");
        }
    }

    #[test]
    fn lucide_svg_resolves_every_bundled_name() {
        for (name, svg) in ALL {
            assert!(lucide_svg(name) == Some(*svg), "{name}");
        }
        assert!(lucide_svg("nope").is_none());
        assert!(file("src/main.RS") == FILE_CODE);
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
