use lucerna_extension::index::{INDEX_VERSION, load_or_rebuild, read, write_install_info};

fn install(root: &std::path::Path) {
    let dir = root.join("installed/ext");
    std::fs::create_dir_all(dir.join("languages/x")).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        "id = \"ext\"\nname = \"Ext\"\nversion = \"1.0.0\"\nschema_version = 1\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("languages/x/config.toml"),
        "name = \"X\"\nline_comments = [\"//\"]\nblock_comment = [\"/* \", \" */\"]\n",
    )
    .unwrap();
    write_install_info(&dir, &Default::default()).unwrap();
}

#[test]
fn old_shape_index_is_rebuilt_with_editing_data() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    install(root);
    // An index written before the editing data existed: no version, no
    // editing fields, and newer than `installed/`.
    let old = r#"{"extensions":{"ext":{"manifest":{"id":"ext","name":"Ext","version":"1.0.0","schema_version":1},
        "info":{"commit":"","version":"","installed_at":0,"source":"registry"},
        "languages":[{"name":"X","dir":"x","grammar":null,"path_suffixes":[],"first_line_pattern":null,"hidden":false,"queries":[]}],
        "themes":[],"icon_themes":[],"snippets":{},"broken":null}}}"#;
    std::fs::write(root.join("index.json"), old).unwrap();
    assert!(read(root).is_none(), "an unversioned index is not trusted");

    let index = load_or_rebuild(root);
    assert_eq!(index.version, INDEX_VERSION);
    let language = &index.extensions["ext"].languages[0];
    assert_eq!(language.line_comments, ["//"]);
    let block = language.block_comment.as_ref().unwrap();
    assert_eq!((block.start.as_str(), block.end.as_str()), ("/*", "*/"));
    // Written back: the next plain read accepts it.
    assert_eq!(read(root).unwrap().version, INDEX_VERSION);
}
