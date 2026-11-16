//! Help: About (`workbench.action.showAboutDialog`): the app's version in
//! a prompt, with a button that copies it and one that opens the
//! third-party license notices. Help: Third-Party Licenses
//! (`workbench.action.openThirdPartyLicenses`) opens those notices in a
//! tab, as Zed's `zed: open licenses` does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, ClipboardItem, PromptLevel, Window};

use crate::{CommandRegistry, OpenMode, status_message};

/// What the prompt says and Copy copies.
const ABOUT_TEXT: &str = concat!("Lucerna ", env!("CARGO_PKG_VERSION"));
const ANSWERS: [&str; 3] = ["OK", "Copy", "Licenses"];
/// The notices `scripts/gen-licenses.py` writes into the app bundle.
const LICENSES_FILE: &str = "licenses.md";
const NO_LICENSES: &str = "Third-party licenses are included only in the packaged app";

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.showAboutDialog",
        "Help: About",
        |_, _, window, cx| show(window, cx),
    );
    commands.register(
        "workbench.action.openThirdPartyLicenses",
        "Help: Third-Party Licenses",
        |wb, _, window, cx| match licenses_path() {
            Some(path) => wb.open_path(Arc::from(path), OpenMode::Pinned, cx),
            None => status_message::show(window.window_handle().window_id(), NO_LICENSES, cx),
        },
    );
}

/// Shows "Lucerna ‹version›" with OK, Copy and Licenses. Copy puts the same
/// text on the clipboard; Licenses opens the notices with the system's
/// default app, since the Welcome screen has no editor to open them in.
pub(crate) fn show(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    let answer = window.prompt(PromptLevel::Info, ABOUT_TEXT, None, &ANSWERS[..], cx);
    cx.spawn(async move |cx| match answer.await {
        Ok(1) => {
            cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(ABOUT_TEXT.into())))
        }
        Ok(2) => cx.update(|cx| match licenses_path() {
            Some(path) => cx.open_with_system(&path),
            None => status_message::show(id, NO_LICENSES, cx),
        }),
        _ => {}
    })
    .detach();
}

/// The shipped notices for the running binary, if they exist.
fn licenses_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    licenses_candidates(&exe).into_iter().find(|p| p.is_file())
}

/// Where the notices sit for the binary at `exe`: in a macOS bundle,
/// `Contents/Resources` beside `Contents/MacOS`; elsewhere, next to the
/// binary.
fn licenses_candidates(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(contents) = dir.parent().filter(|_| dir.ends_with("MacOS")) {
        paths.push(contents.join("Resources").join(LICENSES_FILE));
    }
    paths.push(dir.join(LICENSES_FILE));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_notices_sit_in_resources() {
        let exe = Path::new("/Applications/Lucerna.app/Contents/MacOS/luc");
        assert_eq!(
            licenses_candidates(exe),
            [
                PathBuf::from("/Applications/Lucerna.app/Contents/Resources/licenses.md"),
                PathBuf::from("/Applications/Lucerna.app/Contents/MacOS/licenses.md"),
            ]
        );
    }

    #[test]
    fn loose_binary_notices_sit_beside_it() {
        let exe = Path::new("/opt/lucerna/bin/luc");
        assert_eq!(
            licenses_candidates(exe),
            [PathBuf::from("/opt/lucerna/bin/licenses.md")]
        );
    }
}
