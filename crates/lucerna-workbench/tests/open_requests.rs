use std::path::{Path, PathBuf};

use lucerna_workbench::open_requests::{OpenWindow, Route, file_paths, route};

#[test]
fn file_paths_keeps_file_urls_and_decodes_them() {
    let urls = vec![
        "file:///Users/me/My%20Project/main.rs".to_string(),
        "file:///tmp/".to_string(),
        "https://example.com/x".to_string(),
        "not a url".to_string(),
    ];
    assert_eq!(
        file_paths(&urls),
        vec![
            PathBuf::from("/Users/me/My Project/main.rs"),
            PathBuf::from("/tmp/"),
        ]
    );
}

#[test]
fn route_without_a_window_opens_the_first_one() {
    assert_eq!(route(Path::new("/p/src/a.rs"), None), Route::FirstWindow);
}

#[test]
fn route_inside_the_open_folder_reuses_the_window() {
    let window = OpenWindow::folder(PathBuf::from("/p"));
    assert_eq!(
        route(Path::new("/p/src/a.rs"), Some(&window)),
        Route::ThisWindow
    );
    assert_eq!(route(Path::new("/p"), Some(&window)), Route::ThisWindow);
    // A sibling whose name only starts with the root's name is outside it.
    assert_eq!(
        route(Path::new("/proj/a.rs"), Some(&window)),
        Route::NewProcess
    );
    assert_eq!(
        route(Path::new("/q/a.rs"), Some(&window)),
        Route::NewProcess
    );
}

#[test]
fn route_into_a_single_file_window_matches_only_that_file() {
    let window = OpenWindow::single_file(PathBuf::from("/home/me/notes.txt"));
    assert_eq!(
        route(Path::new("/home/me/notes.txt"), Some(&window)),
        Route::ThisWindow
    );
    assert_eq!(
        route(Path::new("/home/me/other.txt"), Some(&window)),
        Route::NewProcess
    );
}
