//! The image preview tab: images open in it, decode in the background,
//! keep their pixel size, and show a notice with "Open as Text" when they
//! cannot be shown.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{
    AppState, ImagePreview, ImageStatusItem, OpenMode, StatusBarSide, TabItem, Workbench,
    status_text,
};
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

/// The bounds of the element with debug selector `selector`.
fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> gpui::Bounds<gpui::Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"))
}

#[gpui::test]
fn image_that_fits_is_drawn_at_natural_size(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1200.)));
    write_png(&dir.path().join("n.png"), 30, 20);
    let _preview = open(&wb, "n.png", cx);
    draw(cx);
    let image = bounds("image-preview-image", cx);
    assert_eq!(image.size, gpui::size(gpui::px(30.), gpui::px(20.)));
    // The border goes around the image, not into it.
    let frame = bounds("image-preview-box", cx);
    assert!(frame.size.width > image.size.width && frame.size.height > image.size.height);
    assert!(frame.contains(&image.origin));
}

#[gpui::test]
fn large_image_shrinks_to_fit_keeping_aspect(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    cx.simulate_resize(gpui::size(gpui::px(800.), gpui::px(600.)));
    write_png(&dir.path().join("big.png"), 2000, 1500);
    let preview = open(&wb, "big.png", cx);
    draw(cx);
    preview.read_with(cx, |p, _| assert_eq!(p.dims(), Some((2000, 1500))));
    let area = bounds("image-preview", cx);
    let frame = bounds("image-preview-box", cx);
    let image = bounds("image-preview-image", cx);
    // Inside the view, and as large as it can be along one side.
    assert!(frame.left() >= area.left() && frame.right() <= area.right());
    assert!(frame.top() >= area.top() && frame.bottom() <= area.bottom());
    assert!(image.size.width < gpui::px(2000.));
    let (w, h) = (f32::from(image.size.width), f32::from(image.size.height));
    assert!((w / h - 4. / 3.).abs() < 0.01, "aspect kept: {w}×{h}");
    let (aw, ah) = (f32::from(area.size.width), f32::from(area.size.height));
    assert!(
        aw - w < 40. || ah - h < 40.,
        "fills a side: {w}×{h} in {aw}×{ah}"
    );
}

#[gpui::test]
fn closing_tab_frees_image(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let image = preview.read_with(cx, |p, _| p.render_image().expect("decoded"));
    assert!(
        cx.update(|window, _| window.has_image_atlas_entry(&image)),
        "drawn into the atlas"
    );
    let tab = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().expect("a tab").id
    });
    drop(preview);
    wb.update(cx, |wb, cx| wb.update_editor_groups(cx, |m| m.close(tab)));
    draw(cx);
    assert!(
        !cx.update(|window, _| window.has_image_atlas_entry(&image)),
        "the atlas entry is freed"
    );
}

#[gpui::test]
fn svg_rerasters_on_scale_change(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(
        dir.path().join("i.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"/>"#,
    )
    .unwrap();
    let preview = open(&wb, "i.svg", cx);
    let width = |cx: &mut VisualTestContext| {
        preview.read_with(cx, |p, _| {
            i32::from(p.render_image().unwrap().size(0).width)
        })
    };
    cx.simulate_scale_factor_change(1.0);
    draw(cx);
    assert_eq!(width(cx), 40);
    let old = preview.read_with(cx, |p, _| p.render_image().unwrap());
    assert!(cx.update(|window, _| window.has_image_atlas_entry(&old)));
    cx.simulate_scale_factor_change(3.0);
    draw(cx);
    assert_eq!(width(cx), 120);
    assert!(
        !cx.update(|window, _| window.has_image_atlas_entry(&old)),
        "the old raster's atlas entry is freed"
    );
}

#[gpui::test]
fn svg_raster_side_is_capped(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(
        dir.path().join("w.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="10"/>"#,
    )
    .unwrap();
    let preview = open(&wb, "w.svg", cx);
    preview.read_with(cx, |p, _| {
        let size = p.render_image().expect("rasterised").size(0);
        assert!(i32::from(size.width) <= 8192, "{size:?}");
        assert_eq!(p.dims(), Some((20000, 10)));
    });
}

/// The watcher reports a `kind` change of the root-relative `name`; the
/// clock does not move.
fn report(wb: &Entity<Workbench>, name: &str, kind: ChangeKind, cx: &mut VisualTestContext) {
    let change = Change {
        path: Arc::from(Path::new(name)),
        kind,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
}

/// Lets the reload debounce pass and the reload finish.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(ImagePreview::RELOAD_DEBOUNCE);
    draw(cx);
}

fn loads(preview: &Entity<ImagePreview>, cx: &mut VisualTestContext) -> u64 {
    preview.read_with(cx, |p, _| p.loads_started())
}

#[gpui::test]
fn rewrite_reloads_new_dims(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let old = preview.read_with(cx, |p, _| p.render_image().expect("decoded"));
    write_png(&dir.path().join("a.png"), 5, 4);
    report(&wb, "a.png", ChangeKind::Modified, cx);
    // Within the debounce: the old image stays.
    assert_eq!(preview.read_with(cx, |p, _| p.dims()), Some((3, 2)));
    settle(cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((5, 4)));
        assert_eq!(p.error_message(), None);
    });
    assert!(
        !cx.update(|window, _| window.has_image_atlas_entry(&old)),
        "the old image's atlas entry is freed"
    );
}

#[gpui::test]
fn rewrites_in_a_burst_reload_once(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let before = loads(&preview, cx);
    for side in 4..9 {
        write_png(&dir.path().join("a.png"), side, side);
        report(&wb, "a.png", ChangeKind::Modified, cx);
        cx.executor().advance_clock(Duration::from_millis(20));
        draw(cx);
        // No flash to an empty page while the burst lasts.
        assert!(preview.read_with(cx, |p, _| p.is_loaded()));
    }
    settle(cx);
    assert_eq!(loads(&preview, cx), before + 1);
    assert_eq!(preview.read_with(cx, |p, _| p.dims()), Some((8, 8)));
}

#[gpui::test]
fn failed_reload_keeps_old_image(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let before = loads(&preview, cx);
    // Half-written: the PNG signature and nothing more.
    fs::write(dir.path().join("a.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    report(&wb, "a.png", ChangeKind::Modified, cx);
    settle(cx);
    assert_eq!(loads(&preview, cx), before + 1, "the reload ran");
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((3, 2)));
        assert!(p.is_loaded());
        assert_eq!(p.error_message(), None);
    });
}

#[gpui::test]
fn removed_marks_deleted_and_back_reloads(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    assert!(cx.debug_bounds("tab-deleted-0-0").is_none());
    fs::remove_file(dir.path().join("a.png")).unwrap();
    report(&wb, "a.png", ChangeKind::Removed, cx);
    settle(cx);
    preview.read_with(cx, |p, _| {
        assert!(p.is_deleted());
        assert_eq!(p.dims(), Some((3, 2)), "the last image stays");
    });
    assert!(cx.debug_bounds("tab-deleted-0-0").is_some(), "title struck");

    let before = loads(&preview, cx);
    write_png(&dir.path().join("a.png"), 5, 4);
    report(&wb, "a.png", ChangeKind::Added, cx);
    settle(cx);
    assert_eq!(loads(&preview, cx), before + 1);
    preview.read_with(cx, |p, _| {
        assert!(!p.is_deleted());
        assert_eq!(p.dims(), Some((5, 4)));
    });
    assert!(cx.debug_bounds("tab-deleted-0-0").is_none());
}

#[gpui::test]
fn reload_keeps_preview_tab_unpinned(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let (item, pinned) = active_item(&wb, cx);
    assert!(!pinned);
    write_png(&dir.path().join("a.png"), 5, 4);
    report(&wb, "a.png", ChangeKind::Modified, cx);
    settle(cx);
    assert_eq!(preview.read_with(cx, |p, _| p.dims()), Some((5, 4)));
    assert_eq!(active_item(&wb, cx), (item, false));
}

/// An image in a gitignored folder (an agent's screenshot output): the
/// worktree does not report it, the open-file watcher does, and it reloads
/// and is marked deleted all the same.
#[gpui::test]
fn image_in_ignored_folder_reloads(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join(".gitignore"), "out/\n").unwrap();
    fs::create_dir(dir.path().join("out")).unwrap();
    let shot = dir.path().join("out/shot.png");
    write_png(&shot, 3, 2);
    let preview = open(&wb, "out/shot.png", cx);
    assert_eq!(preview.read_with(cx, |p, _| p.dims()), Some((3, 2)));

    write_png(&shot, 5, 4);
    let changed = shot.canonicalize().unwrap();
    wb.update(cx, |wb, cx| {
        wb.on_open_file_changes(std::slice::from_ref(&changed), cx)
    });
    settle(cx);
    assert_eq!(preview.read_with(cx, |p, _| p.dims()), Some((5, 4)));

    fs::remove_file(&shot).unwrap();
    wb.update(cx, |wb, cx| wb.on_open_file_changes(&[changed], cx));
    settle(cx);
    preview.read_with(cx, |p, _| {
        assert!(p.is_deleted());
        assert_eq!(p.dims(), Some((5, 4)));
    });
}

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"/>"#;

/// The items of the active group's tabs and its active tab's index.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (Vec<TabItem>, Option<usize>) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let model = groups.model();
        let group = &model.groups()[model.active_group()];
        let items = group.tabs.iter().map(|t| t.item.clone()).collect();
        let active = group.tabs.iter().position(|t| Some(t.id) == group.active);
        (items, active)
    })
}

#[gpui::test]
fn cmd_shift_v_toggles_svg(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    let svg: Arc<Path> = Arc::from(dir.path().join("i.svg"));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.png")), OpenMode::Pinned, cx)
    });
    let _preview = open(&wb, "i.svg", cx);
    assert_eq!(tabs(&wb, cx).0[1], TabItem::ImagePreview(svg.clone()));

    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    let (items, active) = tabs(&wb, cx);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[1], TabItem::File(svg.clone()));
    assert_eq!(active, Some(1));

    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    let (items, active) = tabs(&wb, cx);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[1], TabItem::ImagePreview(svg));
    assert_eq!(active, Some(1));
}

/// `.SVG` in upper case, as in screenshots from Windows.
#[gpui::test]
fn cmd_shift_v_toggles_upper_case_svg(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("I.SVG"), SVG).unwrap();
    let svg: Arc<Path> = Arc::from(dir.path().join("I.SVG"));
    let _preview = open(&wb, "I.SVG", cx);
    assert_eq!(tabs(&wb, cx).0, vec![TabItem::ImagePreview(svg.clone())]);
    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    assert_eq!(tabs(&wb, cx).0, vec![TabItem::File(svg.clone())]);
    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    assert_eq!(tabs(&wb, cx).0, vec![TabItem::ImagePreview(svg)]);
}

#[gpui::test]
fn cmd_shift_v_noop_on_png(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let _preview = open(&wb, "a.png", cx);
    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    let (items, _) = tabs(&wb, cx);
    assert_eq!(
        items,
        vec![TabItem::ImagePreview(Arc::from(dir.path().join("a.png")))]
    );
}

#[gpui::test]
fn reopen_as_text_activates_existing_source(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    let svg: Arc<Path> = Arc::from(dir.path().join("i.svg"));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("i.svg")), OpenMode::Pinned, cx);
        // The source opens beside the preview, which is not reused.
        wb.open_source(Arc::from(Path::new("i.svg")), OpenMode::Pinned, cx);
    });
    draw(cx);
    let preview = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let model = groups.model();
        let group = &model.groups()[model.active_group()];
        group.tabs[0].id
    });
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.activate(preview))
    });
    draw(cx);
    assert_eq!(
        tabs(&wb, cx),
        (
            vec![
                TabItem::ImagePreview(svg.clone()),
                TabItem::File(svg.clone())
            ],
            Some(0)
        )
    );

    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command("imagePreview.reopenAsText", None, window, cx))
        })
    });
    draw(cx);
    assert_eq!(tabs(&wb, cx), (vec![TabItem::File(svg)], Some(0)));
}

#[gpui::test]
fn open_as_text_button_forces_text(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let file = dir.path().join("c.png");
    // Not an image, and binary: only a forced text load opens it.
    fs::write(&file, b"\x89PNG\r\n\x1a\n\0\0junk").unwrap();
    let preview = open(&wb, "c.png", cx);
    assert!(preview.read_with(cx, |p, _| p.error_message().is_some()));
    let button = bounds("image-open-as-text", cx);
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    let view = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<lucerna_workbench::code_view::CodeView>()
            .ok()
    });
    let view = view.expect("a code view");
    assert_eq!(
        view.read_with(cx, |v, _| v.doc_state().clone()),
        lucerna_workbench::DocState::Ready
    );
}

#[gpui::test]
fn reopen_as_preview_command_and_path_bar_button(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    let svg: Arc<Path> = Arc::from(dir.path().join("i.svg"));
    let _preview = open(&wb, "i.svg", cx);
    let button = bounds("image-preview-toggle-0", cx);
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(tabs(&wb, cx).0, vec![TabItem::File(svg.clone())]);
    let button = bounds("image-preview-toggle-0", cx);
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(tabs(&wb, cx).0, vec![TabItem::ImagePreview(svg)]);
    // No button on a raster image.
    let _png = open(&wb, "a.png", cx);
    assert!(cx.debug_bounds("image-preview-toggle-0").is_none());
}

#[gpui::test]
fn reopen_as_preview_with_path_opens_pinned_image(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    let args = serde_json::json!({ "path": "i.svg" });
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command("imagePreview.reopenAsPreview", Some(&args), window, cx))
        })
    });
    draw(cx);
    let (item, pinned) = active_item(&wb, cx);
    assert_eq!(
        item,
        TabItem::ImagePreview(Arc::from(dir.path().join("i.svg")))
    );
    assert!(pinned);
}

/// Right-clicks the Explorer row of `name` and says whether its menu runs
/// `command`; closes the menu.
fn explorer_menu_has(name: &str, command: &str, cx: &mut VisualTestContext) -> bool {
    let row: &'static str = Box::leak(format!("explorer-row-{name}").into_boxed_str());
    let position = bounds(row, cx).center();
    for event in [true, false] {
        let (button, modifiers) = (gpui::MouseButton::Right, gpui::Modifiers::none());
        if event {
            cx.simulate_event(gpui::MouseDownEvent {
                position,
                modifiers,
                button,
                click_count: 1,
                first_mouse: false,
            });
        } else {
            cx.simulate_event(gpui::MouseUpEvent {
                position,
                modifiers,
                button,
                click_count: 1,
            });
        }
    }
    draw(cx);
    let item: &'static str = Box::leak(format!("context-menu-item-{command}").into_boxed_str());
    let has = cx.debug_bounds(item).is_some();
    cx.simulate_keystrokes("escape");
    draw(cx);
    has
}

#[gpui::test]
fn explorer_menu_has_open_preview_for_svg(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("a.png"), 3, 2);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(std::time::Instant::now() < deadline, "scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    draw(cx);
    assert!(explorer_menu_has(
        "i.svg",
        "imagePreview.reopenAsPreview",
        cx
    ));
    assert!(!explorer_menu_has(
        "a.png",
        "imagePreview.reopenAsPreview",
        cx
    ));
    assert!(!explorer_menu_has("a.png", "markdown.showPreview", cx));
}

/// The text of the image status item, or `None` when it shows nothing.
fn status_image(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    let shown = cx.debug_bounds("status-image").is_some();
    let text = cx.update(|_, cx| {
        wb.read(cx)
            .status_items(StatusBarSide::Right)
            .iter()
            .find_map(|v| v.clone().downcast::<ImageStatusItem>().ok())
            .expect("the image status item")
            .read(cx)
            .text(cx)
    });
    assert_eq!(shown, text.is_some(), "drawn exactly when it has text");
    text
}

#[gpui::test]
fn status_bar_shows_dims_and_size(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    open(&wb, "a.png", cx);
    let len = |name: &str| fs::metadata(dir.path().join(name)).unwrap().len();
    assert_eq!(
        status_image(&wb, cx),
        Some(status_text(Some((3, 2)), len("a.png")))
    );
    assert!(cx.debug_bounds("status-cursor").is_none());

    write_png(&dir.path().join("a.png"), 5, 4);
    report(&wb, "a.png", ChangeKind::Modified, cx);
    settle(cx);
    assert_eq!(
        status_image(&wb, cx),
        Some(status_text(Some((5, 4)), len("a.png")))
    );
}

#[gpui::test]
fn status_bar_shows_only_size_for_an_error(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("c.png"), b"not an image").unwrap();
    open(&wb, "c.png", cx);
    assert_eq!(status_image(&wb, cx).as_deref(), Some("12 B"));

    // Too many pixels: a valid PNG shell of 20000 x 12000 with no pixel data.
    let header: [u8; 65] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x4e, 0x20, 0x00, 0x00, 0x2e, 0xe0, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf6,
        0x4c, 0x83, 0x3f, 0x00, 0x00, 0x00, 0x08, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x03, 0x00,
        0x00, 0x00, 0x00, 0x01, 0x48, 0x06, 0x89, 0xd2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
        0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    fs::write(dir.path().join("huge.png"), header).unwrap();
    let preview = open(&wb, "huge.png", cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(
            p.error_message().as_deref(),
            Some("The image is too large to preview (20000 \u{d7} 12000).")
        );
    });
    assert_eq!(status_image(&wb, cx).as_deref(), Some("65 B"));
}

#[gpui::test]
fn status_bar_shows_only_size_for_too_large_bytes(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let file = fs::File::create(dir.path().join("big.png")).unwrap();
    file.set_len(21 * 1024 * 1024).unwrap();
    open(&wb, "big.png", cx);
    assert_eq!(status_image(&wb, cx).as_deref(), Some("21.0 MB"));
}

/// A `w`×`h` JPEG whose EXIF says Orientation=6 (rotate 90° clockwise), so
/// it shows as `h`×`w`.
fn write_rotated_jpeg(path: &Path, w: u32, h: u32) {
    let mut jpeg = Vec::new();
    image::RgbImage::from_pixel(w, h, image::Rgb([30, 200, 30]))
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new(&mut jpeg))
        .unwrap();
    // APP1: "Exif\0\0", a big-endian TIFF header, one IFD entry
    // (0x0112 Orientation, SHORT, 1, value 6), no next IFD.
    let mut app1 = vec![0xFF, 0xE1, 0x00, 0x22];
    app1.extend_from_slice(b"Exif\0\0MM\0\x2A\0\0\0\x08");
    app1.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0]);
    assert_eq!(app1.len(), 2 + 0x22);
    // After SOI.
    jpeg.splice(2..2, app1);
    fs::write(path, jpeg).unwrap();
}

#[gpui::test]
fn exif_rotated_jpeg_is_portrait(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1200.)));
    write_rotated_jpeg(&dir.path().join("r.jpg"), 40, 20);
    let preview = open(&wb, "r.jpg", cx);
    draw(cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((20, 40)));
        let size = p.render_image().expect("decoded").size(0);
        assert_eq!((i32::from(size.width), i32::from(size.height)), (20, 40));
    });
    let image = bounds("image-preview-image", cx);
    assert_eq!(image.size, gpui::size(gpui::px(20.), gpui::px(40.)));
}

/// An animated GIF of `frames` frames of `w`×`h`.
fn write_gif(path: &Path, w: u32, h: u32, frames: u8) {
    let mut gif = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
        for i in 0..frames {
            let buffer = image::RgbaImage::from_pixel(w, h, image::Rgba([i * 40, 0, 0, 255]));
            encoder.encode_frame(image::Frame::new(buffer)).unwrap();
        }
    }
    fs::write(path, gif).unwrap();
}

#[gpui::test]
fn small_animated_gif_keeps_its_frames(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    write_gif(&dir.path().join("g.gif"), 4, 3, 3);
    let preview = open(&wb, "g.gif", cx);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.dims(), Some((4, 3)));
        assert_eq!(p.render_image().expect("decoded").frame_count(), 3);
    });
}

/// A file that becomes too large on a reload shows its notice; the old
/// image is freed.
#[gpui::test]
fn too_large_reload_replaces_image(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    let preview = open(&wb, "a.png", cx);
    let old = preview.read_with(cx, |p, _| p.render_image().expect("decoded"));
    assert!(cx.update(|window, _| window.has_image_atlas_entry(&old)));
    let file = fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("a.png"))
        .unwrap();
    file.set_len(21 * 1024 * 1024).unwrap();
    drop(file);
    report(&wb, "a.png", ChangeKind::Modified, cx);
    settle(cx);
    preview.read_with(cx, |p, _| {
        assert!(!p.is_loaded());
        assert_eq!(
            p.error_message().as_deref(),
            Some("The image is too large to preview (21.0 MB).")
        );
    });
    assert!(
        !cx.update(|window, _| window.has_image_atlas_entry(&old)),
        "the old image's atlas entry is freed"
    );
}

/// An SVG shown both as an edited text tab and as an image tab: its dirty
/// document lingers in Documents, yet deleting the file shows no Documents
/// bar while the image tab is active (image tabs have none).
#[gpui::test]
fn image_tab_has_no_disk_bar(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("i.svg"), SVG).unwrap();
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("i.svg")), OpenMode::Pinned, cx);
        wb.open_source(Arc::from(Path::new("i.svg")), OpenMode::Pinned, cx);
    });
    draw(cx);
    cx.simulate_input("x");
    draw(cx);
    let svg: Arc<Path> = Arc::from(dir.path().join("i.svg"));
    let image = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let model = groups.model();
        model.groups()[model.active_group()].tabs[0].id
    });
    fs::remove_file(&svg).unwrap();
    report(&wb, "i.svg", ChangeKind::Removed, cx);
    settle(cx);
    let notice = cx.update(|_, cx| wb.read(cx).documents().read(cx).disk_notice(&svg));
    assert_eq!(notice, Some(lucerna_workbench::DiskNotice::Deleted));
    assert!(
        cx.debug_bounds("conflict-bar-0").is_some(),
        "on the text tab"
    );

    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.activate(image))
    });
    draw(cx);
    assert_eq!(
        tabs(&wb, cx),
        (
            vec![TabItem::ImagePreview(svg.clone()), TabItem::File(svg)],
            Some(0)
        )
    );
    assert!(
        cx.debug_bounds("conflict-bar-0").is_none(),
        "none on the image tab"
    );
}
