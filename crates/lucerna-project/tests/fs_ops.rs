use lucerna_project::fs_ops::*;
use std::io::ErrorKind;

#[test]
fn validate_rejects_bad_names() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    std::fs::write(p.join("a.rs"), "").unwrap();
    let v = |n: &str| validate_new_name(p, n, None, |q| q.exists());
    assert!(matches!(v(""), Err(NameError::Empty)));
    assert!(matches!(v("   "), Err(NameError::Empty)));
    assert!(matches!(v("/a"), Err(NameError::LeadingSlash)));
    assert!(matches!(v("a/../b"), Err(NameError::Invalid(_))));
    match v("a.rs") {
        Err(NameError::Exists(n)) => assert_eq!(n, "a.rs"),
        other => panic!("{other:?}"),
    }
    assert_eq!(v("x/y/z.rs").unwrap(), p.join("x/y/z.rs"));
    assert_eq!(
        NameError::Exists("a.rs".into()).to_string(),
        "A file or folder a.rs already exists at this location. Please choose a different name."
    );
}

#[test]
fn create_file_makes_parents_and_never_overwrites() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x/y/z.rs");
    create_file(&f).unwrap();
    assert!(f.exists());
    std::fs::write(&f, "keep").unwrap();
    let e = create_file(&f).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "keep");
    let dir = d.path().join("m/n");
    create_dir(&dir).unwrap();
    assert_eq!(
        create_dir(&dir).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
}

#[test]
fn rename_refuses_existing_target() {
    let d = tempfile::tempdir().unwrap();
    let (a, b) = (d.path().join("a"), d.path().join("b"));
    std::fs::write(&a, "1").unwrap();
    std::fs::write(&b, "2").unwrap();
    assert_eq!(rename(&a, &b).unwrap_err().kind(), ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "2");
    rename(&a, &d.path().join("sub/c")).unwrap();
    assert!(d.path().join("sub/c").exists());
}

#[test]
fn case_only_rename_is_allowed() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("readme.md");
    std::fs::write(&r, "x").unwrap();
    let t = validate_new_name(d.path(), "README.md", Some(&r), |p| p.exists()).unwrap();
    rename(&r, &t).unwrap();
    let names: Vec<_> = std::fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["README.md"]);
}

#[test]
fn trash_removes_entry() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("t.txt");
    std::fs::write(&f, "x").unwrap();
    match trash(&f) {
        Ok(()) => assert!(!f.exists()),
        Err(e) => eprintln!("skipping: no Trash available here: {}", e.0),
    }
}

#[test]
fn delete_permanently_removes_dir_tree() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("t/u");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f"), "x").unwrap();
    delete_permanently(&d.path().join("t")).unwrap();
    assert!(!d.path().join("t").exists());
    let f = d.path().join("f");
    std::fs::write(&f, "x").unwrap();
    delete_permanently(&f).unwrap();
    assert!(!f.exists());
}

#[test]
fn stem_range_cases() {
    assert_eq!(stem_range("foo.rs", false), 0..3);
    assert_eq!(stem_range("foo.tar.gz", false), 0..7);
    assert_eq!(stem_range(".gitignore", false), 0..10);
    assert_eq!(stem_range("a.b", true), 0..3);
}
