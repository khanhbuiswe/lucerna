use lucerna_workbench::markdown::*;

#[test]
fn table_with_alignment() {
    let md = parse_markdown_document("| a | b | c |\n|:--|:-:|--:|\n| 1 | **2** | |\n", None);
    let Block::Table { aligns, head, rows } = &md.blocks()[0] else {
        panic!()
    };
    assert_eq!(aligns, &[Align::Left, Align::Center, Align::Right]);
    assert_eq!(head.len(), 3);
    assert_eq!(
        rows[0][1],
        vec![Inline::Strong(vec![Inline::Text("2".into())])]
    );
    assert!(rows[0][2].is_empty());
}

#[test]
fn nested_task_list_and_start() {
    let md = parse_markdown_document("3. x\n4. y\n\n- [ ] a\n  - [x] b\n", None);
    let Block::List { start, .. } = &md.blocks()[0] else {
        panic!()
    };
    assert_eq!(*start, Some(3));
    let Block::List { start: None, items } = &md.blocks()[1] else {
        panic!()
    };
    assert_eq!(items[0].checked, Some(false));
    let Block::List { items: inner, .. } = &items[0].blocks[1] else {
        panic!()
    };
    assert_eq!(inner[0].checked, Some(true));
}

#[test]
fn block_and_inline_images() {
    let md = parse_markdown_document("![logo](img/a.png)\n\ntext ![i](b.svg) end\n", None);
    assert_eq!(
        md.blocks()[0],
        Block::Image {
            url: "img/a.png".into(),
            alt: "logo".into()
        }
    );
    let Block::Paragraph(inl) = &md.blocks()[1] else {
        panic!()
    };
    assert!(inl.contains(&Inline::Image {
        url: "b.svg".into(),
        alt: "i".into()
    }));
}

#[test]
fn front_matter_hidden_and_alert() {
    let md = parse_markdown_document("---\ntitle: x\n---\n\n> [!NOTE]\n> hi\n\n~~old~~\n", None);
    assert!(matches!(
        md.blocks()[0],
        Block::Alert {
            kind: AlertKind::Note,
            ..
        }
    ));
    let Block::Paragraph(inl) = &md.blocks()[1] else {
        panic!()
    };
    assert!(matches!(inl[0], Inline::Strike(_)));
}

#[test]
fn root_lines_cover_blocks() {
    let src = "# T\n\npara\nline2\n\n```rs\na\nb\n```\n\n- a\n\n  b\n- c\n";
    let md = parse_markdown_document(src, None);
    assert_eq!(md.root_lines(), &[0..1, 2..4, 5..9, 10..14]);
    assert_eq!(md.line_count(), 14);
}

#[test]
fn root_lines_crlf_and_no_trailing_newline() {
    let md = parse_markdown_document("# T\r\n\r\npara", None);
    assert_eq!(md.root_lines(), &[0..1, 2..3]);
    assert_eq!(md.line_count(), 3);
}

#[test]
fn code_index_start_counts_nested_code() {
    let md = parse_markdown_document("```\na\n```\n\n> ```\n> b\n> ```\n\n```\nc\n```\n", None);
    assert_eq!(
        (0..3).map(|i| md.code_index_start(i)).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn hover_still_cuts_at_limit() {
    let big = "a".repeat(MAX_MARKDOWN_BYTES + 10);
    assert!(
        matches!(&parse_markdown(&big, None).blocks()[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text(t) if t.len() == MAX_MARKDOWN_BYTES))
    );
    assert!(
        matches!(&parse_markdown_document(&big, None).blocks()[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text(t) if t.len() == MAX_MARKDOWN_BYTES + 10))
    );
}
