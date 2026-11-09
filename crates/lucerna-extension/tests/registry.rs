use lucerna_extension::disk::{MIN_FREE_BYTES, free_bytes};
use lucerna_extension::process::CancelToken;
use lucerna_extension::registry::{
    Registry, RegistryEntry, is_stale, load_cached, parse_parts, refresh,
};
use lucerna_extension::test_support::{FakeEntry, fake_registry, fake_repo, registry_update};
use std::time::{Duration, SystemTime};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/registry/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn parse_parts_joins_three_sources() {
    let (entries, warnings) = parse_parts(
        &fixture("extensions.toml"),
        &fixture("gitmodules"),
        &fixture("ls-tree.txt"),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let ids: Vec<_> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        ["0x96f", "0xtz", "1984-theme", "42nd-theme", "8008-theme"]
    );
    let e = &entries[3];
    assert_eq!(e.version, "0.1.0");
    assert_eq!(e.url, "https://github.com/fawkes42/42nd.git");
    assert_eq!(e.commit, format!("{:040x}", 4));
    assert_eq!(e.path.as_deref(), Some("zed"));
    assert_eq!(entries[0].path, None);
    assert_eq!(entries[0].commit, format!("{:040x}", 1));
}

#[test]
fn parse_parts_skips_entry_without_commit_with_warning() {
    let toml = "[a]\nsubmodule = \"extensions/a\"\nversion = \"1.0.0\"\n\n[b]\nsubmodule = \"extensions/b\"\nversion = \"1.0.0\"\n\n[c]\nsubmodule = \"extensions/c\"\nversion = \"1.0.0\"\n";
    let gm = "[submodule \"extensions/a\"]\n\tpath = extensions/a\n\turl = git@github.com:o/a.git\n\n[submodule \"extensions/b\"]\n\tpath = extensions/b\n\turl = https://x/b\n";
    let lt = format!(
        "160000 commit {:040x}\textensions/a\n160000 commit {:040x}\textensions/c\n",
        1, 3
    );
    let (entries, warnings) = parse_parts(toml, gm, &lt);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "a");
    assert_eq!(entries[0].url, "https://github.com/o/a");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.contains(&"b: no commit in tree, skipped".to_string()));
    assert!(warnings.contains(&"c: no url in .gitmodules, skipped".to_string()));
}

fn two_extensions() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (r1, _) = fake_repo(&[("extension.toml", "id = \"one\"\n")]);
    let (r2, _) = fake_repo(&[("zed/extension.toml", "id = \"two\"\n")]);
    let reg = fake_registry(&[
        FakeEntry::new("one", "0.1.0", &r1),
        FakeEntry::new("two", "0.2.0", &r2).path("zed"),
    ]);
    (reg, r1, r2)
}

#[test]
fn refresh_from_file_registry() {
    let (reg, r1, _r2) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let remote = format!("file://{}", reg.display());
    let (registry, warnings) = refresh(
        root.path(),
        &remote,
        true,
        &CancelToken::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(registry.entries.len(), 2);
    let one = &registry.entries[0];
    assert_eq!(one.id, "one");
    assert_eq!(one.version, "0.1.0");
    assert_eq!(one.url, format!("file://{}", r1.display()));
    assert_eq!(one.commit.len(), 40);
    assert_eq!(registry.entries[1].path.as_deref(), Some("zed"));
    assert!(root.path().join("registry.json").exists());
    let cached = load_cached(root.path()).unwrap();
    assert_eq!(cached.entries, registry.entries);
}

#[test]
fn refresh_twice_fetches_updates() {
    let (reg, r1, _r2) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let remote = format!("file://{}", reg.display());
    let cancel = CancelToken::default();
    let (first, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    let old_commit = first.entries[0].commit.clone();

    lucerna_extension::test_support::add_commit(&r1, &[("extra.txt", "x")]);
    registry_update(&reg, "one", "0.1.1");

    let (second, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    assert_eq!(second.entries[0].version, "0.1.1");
    assert_ne!(second.entries[0].commit, old_commit);
}

#[test]
fn refresh_keeps_name_and_description_when_commit_unchanged() {
    let (reg, _r1, _r2) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let remote = format!("file://{}", reg.display());
    let cancel = CancelToken::default();
    let (mut first, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    first.entries[0].name = Some("One".into());
    first.entries[0].description = Some("desc".into());
    std::fs::write(
        root.path().join("registry.json"),
        serde_json::to_vec(&first).unwrap(),
    )
    .unwrap();
    let (second, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    assert_eq!(second.entries[0].name.as_deref(), Some("One"));
    assert_eq!(second.entries[0].description.as_deref(), Some("desc"));
    assert_eq!(second.entries[1].name, None);
}

#[test]
fn corrupt_registry_json_discarded() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path().join("registry.json");
    std::fs::write(&p, "{ not json").unwrap();
    assert!(load_cached(root.path()).is_none());
    assert!(!p.exists());
    assert!(load_cached(root.path()).is_none());
}

#[test]
fn stale_after_24h() {
    let now = SystemTime::now();
    let mk = |age: Duration| Registry {
        fetched_at: now - age,
        entries: Vec::<RegistryEntry>::new(),
    };
    assert!(!is_stale(&mk(Duration::from_secs(23 * 3600)), now));
    assert!(is_stale(&mk(Duration::from_secs(24 * 3600 + 1)), now));
}

#[test]
fn free_bytes_reports_something() {
    let d = tempfile::tempdir().unwrap();
    assert!(free_bytes(d.path()).unwrap() > 0);
    assert_eq!(MIN_FREE_BYTES, 1_288_490_189);
}

#[test]
fn refresh_heals_corrupt_registry_clone() {
    let (reg, _r1, _r2) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let remote = format!("file://{}", reg.display());
    let cancel = CancelToken::default();
    refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    std::fs::write(root.path().join("registry/.git/HEAD"), "").unwrap();
    let (r, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    assert_eq!(r.entries.len(), 2);
}

#[test]
fn refresh_rejects_option_like_and_local_remotes() {
    let (reg, _, _) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("PWNED");
    let cancel = CancelToken::default();
    for (remote, allow) in [
        (
            format!("--upload-pack=touch {}; git-upload-pack", marker.display()),
            true,
        ),
        (format!("file://{}", reg.display()), false),
        (reg.display().to_string(), false),
        ("http://example.com/r".to_string(), true),
    ] {
        let r = refresh(root.path(), &remote, allow, &cancel, &mut Vec::new());
        assert!(r.is_err(), "{remote}");
    }
    assert!(!marker.exists(), "the injected command ran");
    assert!(!root.path().join("registry").exists());
}

#[test]
fn concurrent_refreshes_all_succeed_and_keep_the_clone() {
    let (reg, _r1, _r2) = two_extensions();
    let remote = format!("file://{}", reg.display());
    for round in 0..3 {
        let root = tempfile::tempdir().unwrap();
        if round > 0 {
            // Also race incremental updates of an existing clone.
            refresh(
                root.path(),
                &remote,
                true,
                &CancelToken::default(),
                &mut Vec::new(),
            )
            .unwrap();
        }
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        refresh(
                            root.path(),
                            &remote,
                            true,
                            &CancelToken::default(),
                            &mut Vec::new(),
                        )
                        .map(|(r, _)| r.entries.len())
                        .map_err(|e| e.to_string())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for r in &results {
            assert_eq!(r.as_ref().ok(), Some(&2), "round {round}: {results:?}");
        }
        assert!(root.path().join("registry/.git").is_dir());
    }
}

#[test]
fn unreachable_remote_keeps_a_healthy_clone() {
    let (reg, _r1, _r2) = two_extensions();
    let root = tempfile::tempdir().unwrap();
    let remote = format!("file://{}", reg.display());
    let cancel = CancelToken::default();
    refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    // The remote goes away (a network failure, as far as git can tell).
    let moved = reg.with_file_name("registry-gone");
    std::fs::rename(&reg, &moved).unwrap();
    assert!(refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).is_err());
    assert!(root.path().join("registry/.git").is_dir());
    // It is still usable once the remote is back.
    std::fs::rename(&moved, &reg).unwrap();
    let (r, _) = refresh(root.path(), &remote, true, &cancel, &mut Vec::new()).unwrap();
    assert_eq!(r.entries.len(), 2);
}
