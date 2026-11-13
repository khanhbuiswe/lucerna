//! The line ↔ scroll mapping of the Markdown preview (spec §5.5): a source
//! line maps to the last block starting at or before it, plus how far into
//! that block's lines (up to the next block) it is.

use std::ops::Range;

use lucerna_workbench::{item_to_line, line_to_item};

const BLOCKS: [Range<u32>; 2] = [0..2, 4..10];

#[test]
fn before_first_block() {
    assert_eq!(line_to_item(&[3..5, 7..9], 10, 1.), (0, 0.));
}

#[test]
fn inside_block_fraction() {
    assert_eq!(line_to_item(&BLOCKS, 10, 7.), (1, 0.5));
}

#[test]
fn blank_line_belongs_to_previous() {
    assert_eq!(line_to_item(&BLOCKS, 10, 3.), (0, 0.75));
}

#[test]
fn after_last_block() {
    let (item, fraction) = line_to_item(&[0..2, 4..10], 20, 20.);
    assert_eq!(item, 1);
    assert!((0.999..1.).contains(&fraction), "{fraction}");
}

#[test]
fn empty_doc() {
    assert_eq!(line_to_item(&[], 0, 5.), (0, 0.));
    assert_eq!(item_to_line(&[], 0, 0, 0.5), 0.);
}

#[test]
fn roundtrip() {
    let blocks = [0..3, 5..6, 6..20, 24..25, 30..40, 41..42];
    let line_count = 50;
    // A fixed sequence of lines in [0, 50).
    let mut seed = 7u32;
    for _ in 0..50 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let line = (seed >> 8) as f32 / (1 << 24) as f32 * line_count as f32;
        let (item, fraction) = line_to_item(&blocks, line_count, line);
        let back = item_to_line(&blocks, line_count, item, fraction);
        assert!(
            (back - line).abs() < 1e-3,
            "{line} → ({item}, {fraction}) → {back}"
        );
    }
}
