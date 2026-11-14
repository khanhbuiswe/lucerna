//! Resolving the preview's image and link URLs against the previewed file
//! (spec §5.6): images confined to the workspace, links sorted by what
//! they open, and GitHub-style heading slugs.

use std::fs;
use std::path::{Path, PathBuf};

use lucerna_text::Point;
use lucerna_workbench::markdown::parse_markdown_document;
use lucerna_workbench::{
    ImageContext, ImageSrc, LinkAction, MAX_IMAGE_BYTES, heading_slugs, resolve_image, resolve_link,
};
use tempfile::TempDir;

/// A workspace `root/` with `docs/file.md` in it, and a folder `outside/`
/// next to it.
struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        fs::create_dir_all(root.join("docs/img")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("docs/file.md"), "# x\n").unwrap();
        Fixture {
            _dir: dir,
            root,
            outside,
        }
    }

    /// The context of `docs/file.md`, inside the root.
    fn ctx(&self) -> ImageContext {
        ImageContext::for_file(&self.root.join("docs/file.md"), &self.root)
    }

    fn write(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::write(&path, b"img").unwrap();
        path.canonicalize().unwrap()
    }
}

#[test]
fn relative_image() {
    let f = Fixture::new();
    let png = f.write("docs/img/a.png");
    assert_eq!(
        resolve_image("img/a.png", &f.ctx()),
        ImageSrc::Local(png.clone())
    );
    assert_eq!(
        resolve_image("./img/../img/a.png", &f.ctx()),
        ImageSrc::Local(png)
    );
}

#[test]
fn root_absolute_image() {
    let f = Fixture::new();
    let png = f.write("logo.png");
    assert_eq!(resolve_image("/logo.png", &f.ctx()), ImageSrc::Local(png));
}

#[test]
fn percent_decoded() {
    let f = Fixture::new();
    let png = f.write("docs/img/my pic.png");
    assert_eq!(
        resolve_image("img/my%20pic.png", &f.ctx()),
        ImageSrc::Local(png)
    );
}

#[test]
fn image_outside_root_is_blocked() {
    let f = Fixture::new();
    fs::write(f.outside.join("x.png"), b"img").unwrap();
    assert_eq!(
        resolve_image("../../outside/x.png", &f.ctx()),
        ImageSrc::Blocked("outside")
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(f.outside.join("x.png"), f.root.join("docs/link.png")).unwrap();
        assert_eq!(
            resolve_image("link.png", &f.ctx()),
            ImageSrc::Blocked("outside")
        );
    }
}

#[test]
fn missing_image() {
    let f = Fixture::new();
    assert_eq!(
        resolve_image("img/none.png", &f.ctx()),
        ImageSrc::Blocked("missing")
    );
}

#[test]
fn remote_image_blocked() {
    let f = Fixture::new();
    for url in [
        "https://a.dev/x.png",
        "http://a.dev/x.png",
        "data:image/png;base64,AAAA",
    ] {
        assert_eq!(
            resolve_image(url, &f.ctx()),
            ImageSrc::Blocked("remote"),
            "{url}"
        );
    }
}

#[test]
fn too_large_image() {
    let f = Fixture::new();
    let file = fs::File::create(f.root.join("docs/big.png")).unwrap();
    file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
    assert_eq!(
        resolve_image("big.png", &f.ctx()),
        ImageSrc::Blocked("too-large")
    );
    file.set_len(MAX_IMAGE_BYTES).unwrap();
    assert!(matches!(
        resolve_image("big.png", &f.ctx()),
        ImageSrc::Local(_)
    ));
}

#[test]
fn loose_file_allows_its_dir() {
    let f = Fixture::new();
    fs::write(f.outside.join("loose.md"), "x").unwrap();
    fs::write(f.outside.join("x.png"), b"img").unwrap();
    let ctx = ImageContext::for_file(&f.outside.join("loose.md"), &f.root);
    assert_eq!(ctx.allowed, vec![f.outside.clone()]);
    assert_eq!(
        resolve_image("x.png", &ctx),
        ImageSrc::Local(f.outside.join("x.png").canonicalize().unwrap())
    );
    // The workspace is not the loose file's to show.
    let logo = f.write("logo.png");
    assert!(logo.exists());
    assert_eq!(
        resolve_image("../root/logo.png", &ctx),
        ImageSrc::Blocked("outside")
    );
    // Inside the root, the root is allowed.
    assert_eq!(f.ctx().allowed, vec![f.root.clone()]);
}

#[test]
fn link_kinds() {
    let base = Path::new("/w/docs");
    let root = Path::new("/w");
    let at = |row, column| Point { row, column };
    let cases = [
        ("https://a.dev/x", LinkAction::Web("https://a.dev/x".into())),
        ("http://a.dev", LinkAction::Web("http://a.dev".into())),
        ("mailto:a@b.dev", LinkAction::Web("mailto:a@b.dev".into())),
        ("#install", LinkAction::Anchor("install".into())),
        ("#t%E1%BB%95ng-quan", LinkAction::Anchor("tổng-quan".into())),
        (
            "other.md",
            LinkAction::Markdown {
                path: "/w/docs/other.md".into(),
                slug: None,
            },
        ),
        (
            "../README.markdown#usage",
            LinkAction::Markdown {
                path: "/w/README.markdown".into(),
                slug: Some("usage".into()),
            },
        ),
        (
            "/guide/a%20b.md",
            LinkAction::Markdown {
                path: "/w/guide/a b.md".into(),
                slug: None,
            },
        ),
        (
            "file:///x/y.md",
            LinkAction::Markdown {
                path: "/x/y.md".into(),
                slug: None,
            },
        ),
        (
            "other.md#L10",
            LinkAction::Source {
                path: "/w/docs/other.md".into(),
                point: at(9, 0),
            },
        ),
        (
            "../src/main.rs#L3,5",
            LinkAction::Source {
                path: "/w/src/main.rs".into(),
                point: at(2, 4),
            },
        ),
        (
            "file:///a/b.rs#L7",
            LinkAction::Source {
                path: "/a/b.rs".into(),
                point: at(6, 0),
            },
        ),
        ("../Cargo.toml", LinkAction::File("/w/Cargo.toml".into())),
        (
            "./img/a.png?raw=1",
            LinkAction::File("/w/docs/img/a.png".into()),
        ),
        ("javascript:alert(1)", LinkAction::None),
        ("command:foo", LinkAction::None),
        ("", LinkAction::None),
    ];
    for (url, want) in cases {
        assert_eq!(resolve_link(url, base, root), want, "{url}");
    }
}

#[test]
fn slugs_github_style() {
    let blocks = parse_markdown_document(
        "# Hello, World!\n\npara\n\n## A\n\n## A\n\n### Tổng quan\n\n## snake_case `code`\n",
        None,
    );
    assert_eq!(
        heading_slugs(&blocks),
        vec![
            ("hello-world".to_owned(), 0),
            ("a".to_owned(), 2),
            ("a-1".to_owned(), 3),
            ("tổng-quan".to_owned(), 4),
            ("snake_case-code".to_owned(), 5),
        ]
    );
}
