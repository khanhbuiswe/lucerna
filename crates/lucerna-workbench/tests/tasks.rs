use lucerna_workbench::tasks::{guarded, spawn_guarded, try_spawn_guarded};

#[test]
fn spawn_guarded_returns_value() {
    assert_eq!(spawn_guarded("value", || 7).join().unwrap(), Some(7));
}

#[test]
fn spawn_guarded_catches_panic_returns_none() {
    let handle = spawn_guarded("boom", || -> u32 { panic!("boom") });
    assert_eq!(handle.join().unwrap(), None);
}

#[test]
fn try_spawn_guarded_returns_ok_and_the_value() {
    let handle = try_spawn_guarded("try-value", || 11).expect("thread spawns");
    assert_eq!(handle.join().unwrap(), Some(11));
}

#[test]
fn spawn_guarded_names_the_thread() {
    let name = spawn_guarded("lucerna-test-task", || {
        std::thread::current().name().map(str::to_owned)
    });
    assert_eq!(name.join().unwrap(), Some(Some("lucerna-test-task".into())));
}

#[test]
fn guarded_runs_inline_and_contains_panics() {
    assert_eq!(guarded("inline", || "ok"), Some("ok"));
    assert_eq!(guarded("inline", || -> u8 { panic!("inline boom") }), None);
}
