//! The bundled fonts register with GPUI's text system and resolve by family.
//!
//! `TestAppContext`'s text system is a no-op, so this uses the platform's
//! real one; macOS only creates it on the main thread, hence no libtest
//! harness (see `Cargo.toml`).

use std::collections::HashSet;

use gpui::{FontWeight, TextSystem, font};
use lucerna_workbench::fonts;

fn main() {
    let system = TextSystem::new(gpui_platform::current_platform(true).text_system());
    fonts::register_with(&system);
    let names = system.all_font_names();
    for family in [fonts::EDITOR_FONT_FAMILY, fonts::UI_FONT_FAMILY] {
        assert!(names.iter().any(|n| n == family), "{family} not registered");
        let id = system.resolve_font(&font(family));
        let resolved = system.get_font_for_id(id).expect("resolved font");
        assert_eq!(resolved.family.as_ref(), family, "{family} fell back");
    }

    // Each bundled Lilex style is its own face.
    let lilex = font(fonts::EDITOR_FONT_FAMILY);
    let styles = [
        lilex.clone(),
        lilex.clone().bold(),
        lilex.clone().italic(),
        lilex.bold().italic(),
    ];
    let ids: HashSet<_> = styles.iter().map(|f| system.resolve_font(f)).collect();
    assert_eq!(ids.len(), 4, "Lilex styles share a face: {ids:?}");

    // IBM Plex Sans Medium is not Regular.
    let regular = font(fonts::UI_FONT_FAMILY);
    let medium = gpui::Font {
        weight: FontWeight::MEDIUM,
        ..regular.clone()
    };
    assert_ne!(
        system.resolve_font(&regular),
        system.resolve_font(&medium),
        "IBM Plex Sans Medium resolves to Regular"
    );
    println!("bundled fonts register and resolve: ok");
}
