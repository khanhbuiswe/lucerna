//! The extension commands (spec §5.3).

use gpui::App;

use super::ExtensionService;
use super::page::{CLEAR_CACHE_BUSY, FETCH_FAILED, LOADING_LIST};
use crate::CommandRegistry;

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.view.extensions",
        "View: Show Extensions",
        |wb, _, window, cx| wb.open_extensions(window, cx),
    );
    commands.register(
        "workbench.extensions.action.installExtension",
        "Extensions: Install Extension…",
        |wb, _, window, cx| {
            let Some(service) = cx.try_global::<ExtensionService>() else {
                return;
            };
            if service.registry().is_none() && !service.is_refreshing() {
                ExtensionService::refresh_registry(cx);
            }
            let ids = installable_ids(cx);
            wb.show_extension_ids(ids, window, cx);
        },
    );
    commands.register(
        "workbench.extensions.action.checkForUpdates",
        "Extensions: Check for Updates",
        |_, _, _, cx| {
            if cx.has_global::<ExtensionService>() {
                ExtensionService::refresh_registry(cx);
            }
        },
    );
    commands.register(
        "workbench.extensions.action.updateAllExtensions",
        "Extensions: Update All",
        |_, _, _, cx| {
            if cx.has_global::<ExtensionService>() {
                ExtensionService::update_all(cx);
            }
        },
    );
    commands.register(
        "lucerna.extensions.clearBuildCache",
        "Extensions: Clear Build Cache",
        |wb, _, _, cx| {
            if cx.has_global::<ExtensionService>() && !ExtensionService::clear_build_cache(cx) {
                wb.show_message_toast(CLEAR_CACHE_BUSY, cx);
            }
        },
    );
}

/// The registry ids not installed yet, for the install Quick Open; while
/// the registry is missing, the message it shows instead.
pub(crate) fn installable_ids(cx: &App) -> Result<Vec<String>, &'static str> {
    let Some(service) = cx.try_global::<ExtensionService>() else {
        return Ok(Vec::new());
    };
    let Some(registry) = service.registry() else {
        let failed = !service.is_refreshing() && service.registry_error().is_some();
        return Err(if failed { FETCH_FAILED } else { LOADING_LIST });
    };
    let installed = &service.index().extensions;
    Ok(registry
        .entries
        .iter()
        .filter(|e| !installed.contains_key(&e.id))
        .map(|e| e.id.clone())
        .collect())
}
