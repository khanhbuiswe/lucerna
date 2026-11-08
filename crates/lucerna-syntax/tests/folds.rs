use lucerna_syntax::indent_folds;
use lucerna_text::{Buffer, BufferSnapshot};

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

#[test]
fn indent_folds_python_and_braces() {
    let python = snapshot("def f(x):\n    if x:\n        return 1\n\n    return 2\nprint(f(1))\n");
    assert_eq!(indent_folds(&python, 4, 0..6), vec![0..5, 1..3]);
    // Only headers inside the requested rows; bodies may extend past them.
    assert_eq!(indent_folds(&python, 4, 1..2), vec![1..3]);
    assert_eq!(
        indent_folds(&python, 4, 2..6),
        Vec::<std::ops::Range<u32>>::new()
    );

    let braces = snapshot("fn main() {\n\tlet x = 1;\n\tif x > 0 {\n\t\tx;\n\t}\n}\n");
    assert_eq!(indent_folds(&braces, 4, 0..6), vec![0..5, 2..4]);

    // A tab advances to the next tab stop: with tab_size 4 it is deeper than
    // two spaces, with tab_size 1 it is not.
    let mixed = snapshot("a\n  b\n\tc\n");
    assert_eq!(indent_folds(&mixed, 4, 0..3), vec![0..3, 1..3]);
    assert_eq!(indent_folds(&mixed, 1, 0..3), vec![0..3]);

    // Trailing blank lines are not part of a fold.
    let trailing = snapshot("a:\n  b\n\n\n");
    assert_eq!(indent_folds(&trailing, 2, 0..4), vec![0..2]);
}
