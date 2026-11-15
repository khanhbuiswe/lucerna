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

#[test]
fn case_only_rename_does_not_replace_distinct_file() {
    let d = tempfile::tempdir().unwrap();
    let (a, upper) = (d.path().join("a"), d.path().join("A"));
    std::fs::write(&a, "lower").unwrap();
    std::fs::write(&upper, "upper").unwrap();
    if std::fs::read_to_string(&a).unwrap() != "lower" {
        eprintln!("skipping: case-insensitive file system");
        return;
    }
    assert_eq!(
        rename(&a, &upper).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "lower");
    assert_eq!(std::fs::read_to_string(&upper).unwrap(), "upper");
}

#[cfg(unix)]
#[test]
fn rename_refuses_dangling_symlink_target() {
    let d = tempfile::tempdir().unwrap();
    let (a, link) = (d.path().join("a"), d.path().join("link"));
    std::fs::write(&a, "x").unwrap();
    std::os::unix::fs::symlink(d.path().join("missing"), &link).unwrap();
    assert_eq!(
        rename(&a, &link).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    assert!(a.exists());
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
}

#[test]
fn windows_separators_split_and_reject_traversal() {
    assert!(matches!(
        split_components("a\\..\\..\\x", &['/', '\\']),
        Err(NameError::Invalid(_))
    ));
    assert_eq!(
        split_components("a\\b/c", &['/', '\\']).unwrap(),
        ["a", "b", "c"]
    );
}

#[cfg(unix)]
#[test]
fn delete_permanently_unlinks_symlink_to_dir() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("f"), "x").unwrap();
    let link = d.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    delete_permanently(&link).unwrap();
    assert!(std::fs::symlink_metadata(&link).is_err());
    assert!(target.join("f").exists());
}

#[test]
fn paste_name_increments() {
    let d = std::path::Path::new("/d");
    let taken = |set: &'static [&'static str]| {
        move |p: &std::path::Path| set.iter().any(|n| p == d.join(n))
    };
    let (p, r) = paste_name(d, "a.rs", false, taken(&[]));
    assert_eq!((p, r), (d.join("a.rs"), 0..1));
    let (p, r) = paste_name(d, "a.rs", false, taken(&["a.rs"]));
    assert_eq!((p, r), (d.join("a copy.rs"), 0..6));
    let (p, r) = paste_name(d, "a.rs", false, taken(&["a.rs", "a copy.rs"]));
    assert_eq!((p, r), (d.join("a copy 2.rs"), 0..8));
    let (p, _) = paste_name(
        d,
        "a.rs",
        false,
        taken(&["a.rs", "a copy.rs", "a copy 2.rs"]),
    );
    assert_eq!(p, d.join("a copy 3.rs"));
}

#[test]
fn paste_name_dir_and_dotfile() {
    let d = std::path::Path::new("/d");
    let taken = |n: &'static str| move |p: &std::path::Path| p == d.join(n);
    let (p, r) = paste_name(d, "src", true, taken("src"));
    assert_eq!((p, r), (d.join("src copy"), 0..8));
    let (p, r) = paste_name(d, ".env", false, taken(".env"));
    assert_eq!((p, r), (d.join(".env copy"), 0..9));
    let (p, r) = paste_name(d, "a.tar.gz", false, taken("a.tar.gz"));
    assert_eq!((p, r), (d.join("a.tar copy.gz"), 0..10));
}

#[test]
fn copy_file_never_overwrites() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.txt");
    let b = d.path().join("b.txt");
    std::fs::write(&a, "hello").unwrap();
    copy_entry(&a, &b).unwrap();
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "hello");
    std::fs::write(&a, "new").unwrap();
    let err = copy_entry(&a, &b).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "hello");
}

#[test]
fn copy_dir_recursive() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir_all(src.join("sub/deep")).unwrap();
    std::fs::write(src.join("a.txt"), "a").unwrap();
    std::fs::write(src.join("sub/deep/b.txt"), "b").unwrap();
    let dst = d.path().join("dst");
    copy_entry(&src, &dst).unwrap();
    assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
    assert_eq!(
        std::fs::read_to_string(dst.join("sub/deep/b.txt")).unwrap(),
        "b"
    );
    let err = copy_entry(&src, &dst).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
}

#[cfg(unix)]
#[test]
fn copy_dir_keeps_symlinks() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("t.txt"), "t").unwrap();
    std::os::unix::fs::symlink("t.txt", src.join("link")).unwrap();
    std::os::unix::fs::symlink("missing", src.join("dangling")).unwrap();
    let dst = d.path().join("dst");
    copy_entry(&src, &dst).unwrap();
    for (n, target) in [("link", "t.txt"), ("dangling", "missing")] {
        let m = std::fs::symlink_metadata(dst.join(n)).unwrap();
        assert!(m.file_type().is_symlink(), "{n}");
        assert_eq!(
            std::fs::read_link(dst.join(n)).unwrap(),
            std::path::Path::new(target)
        );
    }
}

#[test]
fn copy_refuses_into_itself() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let err = copy_entry(&src, &src.join("inner")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(!src.join("inner").exists());
}

#[test]
fn is_within_cases() {
    use std::path::Path;
    assert!(is_within(Path::new("/a/b"), Path::new("/a")));
    assert!(is_within(Path::new("/a"), Path::new("/a")));
    assert!(!is_within(Path::new("/ab"), Path::new("/a")));
    assert!(!is_within(Path::new("/a"), Path::new("/a/b")));
}

#[cfg(unix)]
#[test]
fn copy_file_keeps_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("run.sh");
    std::fs::write(&a, "x").unwrap();
    std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o750)).unwrap();
    let b = d.path().join("copy.sh");
    copy_entry(&a, &b).unwrap();
    let mode = std::fs::metadata(&b).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750);
}

#[cfg(unix)]
#[test]
fn copy_refuses_fifo() {
    use std::os::unix::ffi::OsStrExt;
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("pipe");
    let c = std::ffi::CString::new(f.as_os_str().as_bytes()).unwrap();
    // SAFETY: valid NUL-terminated path.
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
        return;
    }
    let err = copy_entry(&f, &d.path().join("out")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(!d.path().join("out").exists());
}
