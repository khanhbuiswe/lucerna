//! The image preview tab: images open in it, decode in the background,
//! keep their pixel size, and show a notice with "Open as Text" when they
//! cannot be shown.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, ImagePreview, OpenMode, TabItem, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Writes a `w`×`h` RGBA PNG to `path`.
fn write_png(path: &Path, w: u32, h: u32) {
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
        .save_with_format(path, image::ImageFormat::Png)
        .unwrap();
}

/// A workbench on a temporary folder holding the 3×2 `a.png`.
fn workbench(cx: &mut TestAppContext) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("a.png"), 3, 2);
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    draw(cx);
    (dir, wb, cx)
}

/// Opens `name` as a preview tab, as the Explorer does, and draws.
fn open_name(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(name)), OpenMode::Preview, cx)
    });
    draw(cx);
}

/// The active tab's body as an image preview.
fn active_preview(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<ImagePreview> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<ImagePreview>()
            .expect("an image preview")
    })
}

/// Opens `name` and returns its loaded (or failed) image preview.
fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> Entity<ImagePreview> {
    open_name(wb, name, cx);
    let preview = active_preview(wb, cx);
    draw(cx);
    preview
}

fn active_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (TabItem, bool) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let tab = groups.model().active_tab().expect("a tab");
        (tab.item.clone(), tab.pinned)
    })
}

#[gpui::test]
fn png_opens_in_image_tab(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let (item, pinned) = active_item(&wb, cx);
    assert_eq!(
        item,
        TabItem::ImagePreview(Arc::from(dir.path().join("a.png")))
    );
    assert!(!pinned);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((3, 2)));
        assert!(p.is_loaded());
        assert_eq!(p.error_message(), None);
        assert_eq!(**p.path(), *dir.path().join("a.png"));
    });
    assert!(cx.debug_bounds("image-preview-image").is_some());
}

#[gpui::test]
fn svg_opens_as_image_with_intrinsic_size(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(
        dir.path().join("i.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"/>"#,
    )
    .unwrap();
    let preview = open(&wb, "i.svg", cx);
    let sf = cx.update(|window, _| window.scale_factor());
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((40, 20)));
        let image = p.render_image().expect("rasterised");
        let size = image.size(0);
        assert_eq!(
            (i32::from(size.width), i32::from(size.height)),
            ((40. * sf).round() as i32, (20. * sf).round() as i32)
        );
    });
}

#[gpui::test]
fn wrong_extension_still_decodes(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    write_png(&dir.path().join("b.jpg"), 3, 2);
    let preview = open(&wb, "b.jpg", cx);
    preview.read_with(cx, |p, _| {
        assert!(p.is_loaded());
        assert_eq!(p.dims(), Some((3, 2)));
    });
}

#[gpui::test]
fn empty_file_shows_error(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("c.png"), b"").unwrap();
    let preview = open(&wb, "c.png", cx);
    preview.read_with(cx, |p, _| {
        assert!(!p.is_loaded());
        assert_eq!(p.dims(), None);
        assert_eq!(
            p.error_message().as_deref(),
            Some("The image could not be loaded.")
        );
        assert_eq!(p.file_bytes(), Some(0));
    });
    assert!(cx.debug_bounds("image-open-as-text").is_some());
}

#[gpui::test]
fn too_large_bytes(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let file = fs::File::create(dir.path().join("big.png")).unwrap();
    file.set_len(21 * 1024 * 1024).unwrap();
    let preview = open(&wb, "big.png", cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(
            p.error_message().as_deref(),
            Some("The image is too large to preview (21.0 MB).")
        );
        assert_eq!(p.file_bytes(), Some(21 * 1024 * 1024));
    });
    assert!(cx.debug_bounds("image-open-as-text").is_some());
}

#[gpui::test]
fn lfs_pointer_message(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(
        dir.path().join("l.png"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 12345\n",
    )
    .unwrap();
    let preview = open(&wb, "l.png", cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(
            p.error_message().as_deref(),
            Some("This image is stored with Git LFS and has not been downloaded.")
        );
    });
}

#[gpui::test]
fn close_during_decode_is_harmless(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workbench(cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.png")), OpenMode::Preview, cx)
    });
    // Drawn once: the view exists and its load has started, not finished.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let preview = active_preview(&wb, cx).downgrade();
    let tab = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().expect("a tab").id
    });
    wb.update(cx, |wb, cx| wb.update_editor_groups(cx, |m| m.close(tab)));
    draw(cx);
    assert!(preview.upgrade().is_none(), "the view is dropped");
    assert!(cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).active_body().is_none()));
}

#[gpui::test]
fn open_with_line_opens_source(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(
        dir.path().join("i.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\">\n<g/>\n<g/>\n</svg>\n",
    )
    .unwrap();
    wb.update(cx, |wb, cx| {
        wb.open_path_at(
            Arc::from(Path::new("i.svg")),
            OpenMode::Preview,
            Point { row: 2, column: 0 },
            cx,
        )
    });
    draw(cx);
    let (item, _) = active_item(&wb, cx);
    assert_eq!(item, TabItem::File(Arc::from(dir.path().join("i.svg"))));
}

#[gpui::test]
fn save_and_revert_on_image_tab_do_nothing(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let before = fs::read(dir.path().join("a.png")).unwrap();
    let _preview = open(&wb, "a.png", cx);
    for command in [
        "workbench.action.files.save",
        "workbench.action.files.revert",
    ] {
        cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
        draw(cx);
    }
    assert_eq!(fs::read(dir.path().join("a.png")).unwrap(), before);
    let open_docs = cx.update(|_, cx| wb.read(cx).documents().read(cx).open_paths());
    assert!(open_docs.is_empty(), "no text document: {open_docs:?}");
    let (item, _) = active_item(&wb, cx);
    assert!(matches!(item, TabItem::ImagePreview(_)));
}
