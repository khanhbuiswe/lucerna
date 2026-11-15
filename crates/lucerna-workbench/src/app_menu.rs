//! The menu bar (spec §9): one table of VS Code command IDs ([`MENUS`]),
//! built into GPUI menus by [`build_menus`] for the platform and the kind
//! of window, run through one action, [`RunCommand`], that goes to the
//! active window. The shortcut beside each item follows the user's keymap.

use std::rc::Rc;

use gpui::private::schemars::JsonSchema;
use gpui::{
    Action, App, AppContext as _, KeyBinding, KeyBindingContextPredicate, Menu, MenuItem, OsAction,
    SharedString, SystemMenuType, WindowHandle, WindowId, actions,
};
use lucerna_settings::{Context as WhenContext, ContextValue, Keymap, Keystroke};
use serde::Deserialize;

use crate::config_files::ConfigState;
use crate::recent::{self, RecentEntry, RecentStore};
use crate::workspace_state::tilde_path;
use crate::{
    AppState, ONBOARDING_COMMANDS, OnboardingPage, WELCOME_COMMANDS, WelcomeView, Workbench,
};

/// Runs the command `id` in the active window: what every command item of
/// the menu bar does.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = lucerna)]
#[schemars(crate = "gpui::private::schemars")]
pub struct RunCommand {
    pub id: SharedString,
}

/// A command item the active window cannot run, such as Save in a
/// Welcome window. No listener handles it, so GPUI reports it unavailable
/// and AppKit, which validates items itself, shows it disabled.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = lucerna)]
#[schemars(crate = "gpui::private::schemars")]
pub struct DisabledCommand {
    pub id: SharedString,
}

/// Opens a File ▸ Open Recent entry in the active window, as choosing it
/// on the Welcome screen or in the Open Recent picker does.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = lucerna, no_json)]
pub struct OpenRecentEntry {
    pub entry: RecentEntry,
}

actions!(lucerna, [Quit, Hide, HideOthers, ShowAll, Minimize, Zoom]);

/// The key context of the menu's own key bindings. No element has it, so
/// those bindings only label the menu items; keys go through `keys.rs`.
const MENU_CONTEXT: &str = "LucernaMenu";

/// How many recent entries File ▸ Open Recent lists before "More…".
const RECENT_MENU_ITEMS: usize = 10;

const ABOUT: &str = "workbench.action.showAboutDialog";
const QUIT: &str = "workbench.action.quit";
const OPEN_RECENT: &str = "workbench.action.openRecent";
const CLEAR_RECENT: &str = "workbench.action.clearRecentFiles";

/// One item of a [`MenuSpec`].
pub enum Entry {
    /// Runs command `id`; hidden when no command has that ID. With `os`,
    /// the platform's own action (the selector AppKit sends to text
    /// fields, for example).
    Command {
        title: &'static str,
        id: &'static str,
        os: Option<OsAction>,
    },
    Separator,
    Submenu {
        title: &'static str,
        entries: &'static [Entry],
    },
    /// File ▸ Open Recent: the recent entries, "More…", "Clear Recently
    /// Opened…".
    RecentSubmenu,
    /// The macOS Services menu.
    Services,
    App(AppAction),
    /// `entry` on `platform` only.
    Only(Platform, &'static Entry),
}

/// One menu of the menu bar, the same on every platform.
pub struct MenuSpec {
    pub title: &'static str,
    pub role: Role,
    pub entries: &'static [Entry],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    App,
    File,
    Edit,
    Selection,
    View,
    Go,
    Window,
    Help,
}

/// A GPUI action of the app or window, not a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

/// The root of the active window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Welcome,
    /// The Get Started page as the root, before a folder is opened.
    Onboarding,
    Workbench,
}

/// What [`build_menus`] depends on besides [`MENUS`].
pub struct BuildInput<'a> {
    /// The registered command IDs ([`Workbench::command_ids`]).
    pub commands: &'a [&'a str],
    pub platform: Platform,
    /// The recent entries that still exist, newest first.
    pub recent: &'a [RecentEntry],
    pub window: WindowKind,
}

const fn command(title: &'static str, id: &'static str) -> Entry {
    Entry::Command {
        title,
        id,
        os: None,
    }
}

const fn os_command(title: &'static str, id: &'static str, os: OsAction) -> Entry {
    Entry::Command {
        title,
        id,
        os: Some(os),
    }
}

const ABOUT_ENTRY: Entry = command("About Lucerna", ABOUT);

/// Lucerna ▸ Settings on macOS, File ▸ Preferences elsewhere.
const SETTINGS_ENTRIES: &[Entry] = &[
    command("Open Settings (JSON)", "workbench.action.openSettingsJson"),
    command(
        "Open Workspace Settings (JSON)",
        "workbench.action.openWorkspaceSettingsFile",
    ),
    command(
        "Open Keyboard Shortcuts (JSON)",
        "workbench.action.openGlobalKeybindingsFile",
    ),
    Entry::Separator,
    command("Color Theme…", "workbench.action.selectTheme"),
    command("File Icon Theme…", "workbench.action.selectIconTheme"),
];

/// Where the app menu's items go on Linux and Windows, as in VS Code.
const OTHER_FILE_TAIL: &[Entry] = &[
    Entry::Separator,
    Entry::Submenu {
        title: "Preferences",
        entries: SETTINGS_ENTRIES,
    },
    Entry::Separator,
    command("Exit", QUIT),
];
const OTHER_HELP_TAIL: &[Entry] = &[Entry::Separator, ABOUT_ENTRY];

/// The menu bar (spec §9.1). Titles follow VS Code's menus.
pub static MENUS: &[MenuSpec] = &[
    MenuSpec {
        title: "Lucerna",
        role: Role::App,
        entries: &[
            ABOUT_ENTRY,
            Entry::Separator,
            Entry::Submenu {
                title: "Settings",
                entries: SETTINGS_ENTRIES,
            },
            command("Extensions", "workbench.view.extensions"),
            Entry::Separator,
            Entry::Services,
            Entry::Separator,
            Entry::App(AppAction::Hide),
            Entry::App(AppAction::HideOthers),
            Entry::App(AppAction::ShowAll),
            Entry::Separator,
            command("Quit Lucerna", QUIT),
        ],
    },
    MenuSpec {
        title: "File",
        role: Role::File,
        entries: &[
            Entry::Only(
                Platform::Mac,
                &command("Open…", "workbench.action.files.openFileFolder"),
            ),
            Entry::Only(
                Platform::Other,
                &command("Open File…", "workbench.action.files.openFile"),
            ),
            Entry::Only(
                Platform::Other,
                &command("Open Folder…", "workbench.action.files.openFolder"),
            ),
            Entry::RecentSubmenu,
            Entry::Separator,
            command("Save", "workbench.action.files.save"),
            command("Save All", "workbench.action.files.saveAll"),
            command(
                "Save without Formatting",
                "workbench.action.files.saveWithoutFormatting",
            ),
            command("Revert File", "workbench.action.files.revert"),
            Entry::Separator,
            command("Close Editor", "workbench.action.closeActiveEditor"),
            command("Close Folder", "workbench.action.closeFolder"),
        ],
    },
    MenuSpec {
        title: "Edit",
        role: Role::Edit,
        entries: &[
            os_command("Undo", "undo", OsAction::Undo),
            os_command("Redo", "redo", OsAction::Redo),
            Entry::Separator,
            os_command("Cut", "editor.action.clipboardCutAction", OsAction::Cut),
            os_command("Copy", "editor.action.clipboardCopyAction", OsAction::Copy),
            os_command(
                "Paste",
                "editor.action.clipboardPasteAction",
                OsAction::Paste,
            ),
            Entry::Separator,
            command("Find", "actions.find"),
            command("Find in Files", "workbench.action.findInFiles"),
            Entry::Separator,
            command("Toggle Line Comment", "editor.action.commentLine"),
        ],
    },
    MenuSpec {
        title: "Selection",
        role: Role::Selection,
        entries: &[
            os_command("Select All", "editor.action.selectAll", OsAction::SelectAll),
            command("Expand Selection", "editor.action.smartSelect.expand"),
            command("Shrink Selection", "editor.action.smartSelect.shrink"),
            Entry::Separator,
            command("Copy Line Up", "editor.action.copyLinesUpAction"),
            command("Copy Line Down", "editor.action.copyLinesDownAction"),
            command("Move Line Up", "editor.action.moveLinesUpAction"),
            command("Move Line Down", "editor.action.moveLinesDownAction"),
        ],
    },
    MenuSpec {
        title: "View",
        role: Role::View,
        entries: &[
            command("Command Palette…", "workbench.action.showCommands"),
            Entry::Separator,
            command("Explorer", "workbench.view.explorer"),
            command("Search", "workbench.view.search"),
            command("Extensions", "workbench.view.extensions"),
            Entry::Separator,
            command("Toggle Sidebar", "workbench.action.toggleSidebarVisibility"),
            command("Split Editor", "workbench.action.splitEditor"),
            command("Open Preview to the Side", "markdown.showPreviewToSide"),
            Entry::Separator,
            command("Word Wrap", "editor.action.toggleWordWrap"),
            command("Zoom In", "editor.action.fontZoomIn"),
            command("Zoom Out", "editor.action.fontZoomOut"),
            Entry::Separator,
            command("Language Server Health", "lucerna.lsp.health"),
            command("Language Server Log", "lucerna.lsp.showLog"),
        ],
    },
    MenuSpec {
        title: "Go",
        role: Role::Go,
        entries: &[
            command("Back", "workbench.action.navigateBack"),
            command("Forward", "workbench.action.navigateForward"),
            Entry::Separator,
            command("Go to File…", "workbench.action.quickOpen"),
            command(
                "Go to Symbol in Workspace…",
                "workbench.action.showAllSymbols",
            ),
            command("Go to Symbol in Editor…", "workbench.action.gotoSymbol"),
            command("Go to Line…", "workbench.action.gotoLine"),
            Entry::Separator,
            command("Go to Definition", "editor.action.revealDefinition"),
            command("Go to Type Definition", "editor.action.goToTypeDefinition"),
            command("Go to Implementations", "editor.action.goToImplementation"),
            command("Go to References", "editor.action.goToReferences"),
            Entry::Separator,
            command("Next Problem", "editor.action.marker.nextInFiles"),
            command("Previous Problem", "editor.action.marker.prevInFiles"),
            Entry::Separator,
            command("Next Change", "workbench.action.editor.nextChange"),
            command("Previous Change", "workbench.action.editor.previousChange"),
        ],
    },
    MenuSpec {
        title: "Window",
        role: Role::Window,
        entries: &[Entry::App(AppAction::Minimize), Entry::App(AppAction::Zoom)],
    },
    MenuSpec {
        title: "Help",
        role: Role::Help,
        entries: &[
            command("Get Started", "lucerna.openOnboarding"),
            Entry::Separator,
            command("Show All Commands", "workbench.action.showCommands"),
            command("Open Logs Folder", "lucerna.openLogsFolder"),
            command(
                "Third-Party Licenses",
                "workbench.action.openThirdPartyLicenses",
            ),
        ],
    },
];

/// The menu bar for `input` (spec §9.2). Commands that are not registered
/// are left out, and so are separators left at an end or next to another.
/// On Linux and Windows the app menu's items move to File and Help. In a
/// Welcome or Get Started window only what that page runs is enabled.
pub fn build_menus(input: &BuildInput) -> Vec<Menu> {
    MENUS
        .iter()
        .filter_map(|spec| {
            let tail = match (spec.role, input.platform) {
                (Role::App, Platform::Other) => return None,
                (Role::File, Platform::Other) => OTHER_FILE_TAIL,
                (Role::Help, Platform::Other) => OTHER_HELP_TAIL,
                _ => &[],
            };
            let items = items(spec.entries.iter().chain(tail), input);
            (!items.is_empty()).then(|| Menu::new(spec.title).items(items))
        })
        .collect()
}

fn items<'a>(entries: impl IntoIterator<Item = &'a Entry>, input: &BuildInput) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for entry in entries {
        push_entry(entry, input, &mut items);
    }
    collapse_separators(items)
}

fn push_entry(entry: &Entry, input: &BuildInput, items: &mut Vec<MenuItem>) {
    match entry {
        Entry::Command { title, id, os } => items.extend(command_item(title, id, *os, input)),
        Entry::Separator => items.push(MenuItem::separator()),
        Entry::Submenu { title, entries } => {
            let children = self::items(entries.iter(), input);
            if !children.is_empty() {
                let disabled = children
                    .iter()
                    .all(|c| matches!(c, MenuItem::Separator) || c.is_disabled());
                items.push(MenuItem::submenu(
                    Menu::new(*title).items(children).disabled(disabled),
                ));
            }
        }
        Entry::RecentSubmenu => items.push(recent_submenu(input)),
        Entry::Services => {
            if input.platform == Platform::Mac {
                items.push(MenuItem::os_submenu("Services", SystemMenuType::Services));
            }
        }
        Entry::App(action) => items.extend(app_item(*action, input.platform)),
        Entry::Only(platform, entry) => {
            if *platform == input.platform {
                push_entry(entry, input, items);
            }
        }
    }
}

/// The item running `id`, or `None` when `id` is not registered.
fn command_item(
    title: &'static str,
    id: &'static str,
    os: Option<OsAction>,
    input: &BuildInput,
) -> Option<MenuItem> {
    if !input.commands.contains(&id) {
        return None;
    }
    let enabled = match input.window {
        WindowKind::Workbench => true,
        WindowKind::Welcome => WELCOME_COMMANDS.contains(&id),
        WindowKind::Onboarding => ONBOARDING_COMMANDS.contains(&id),
    };
    let item = match (os, enabled) {
        (Some(os), true) => MenuItem::os_action(title, RunCommand { id: id.into() }, os),
        (None, true) => MenuItem::action(title, RunCommand { id: id.into() }),
        (Some(os), false) => MenuItem::os_action(title, DisabledCommand { id: id.into() }, os),
        (None, false) => MenuItem::action(title, DisabledCommand { id: id.into() }),
    };
    Some(item.disabled(!enabled))
}

fn app_item(action: AppAction, platform: Platform) -> Option<MenuItem> {
    let mac = platform == Platform::Mac;
    Some(match action {
        AppAction::Hide if mac => MenuItem::action("Hide Lucerna", Hide),
        AppAction::HideOthers if mac => MenuItem::action("Hide Others", HideOthers),
        AppAction::ShowAll if mac => MenuItem::action("Show All", ShowAll),
        AppAction::Hide | AppAction::HideOthers | AppAction::ShowAll => return None,
        AppAction::Minimize => MenuItem::action("Minimize", Minimize),
        AppAction::Zoom => MenuItem::action("Zoom", Zoom),
    })
}

/// File ▸ Open Recent: the first recent entries, then "More…" and "Clear
/// Recently Opened…". The Get Started page cannot open an entry, so its
/// window lists none.
fn recent_submenu(input: &BuildInput) -> MenuItem {
    let home = std::env::home_dir();
    let recent = match input.window {
        WindowKind::Onboarding => &[],
        _ => input.recent,
    };
    let mut children: Vec<MenuItem> = recent
        .iter()
        .take(RECENT_MENU_ITEMS)
        .map(|entry| {
            MenuItem::action(
                tilde_path(&entry.path, home.as_deref()),
                OpenRecentEntry {
                    entry: entry.clone(),
                },
            )
        })
        .collect();
    children.push(MenuItem::separator());
    children.extend(command_item("More…", OPEN_RECENT, None, input));
    children.extend(command_item(
        "Clear Recently Opened…",
        CLEAR_RECENT,
        None,
        input,
    ));
    MenuItem::submenu(Menu::new("Open Recent").items(collapse_separators(children)))
}

/// Drops separators at either end and after another separator.
fn collapse_separators(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        let separator = matches!(item, MenuItem::Separator);
        if separator && out.last().is_none_or(|l| matches!(l, MenuItem::Separator)) {
            continue;
        }
        out.push(item);
    }
    if matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

/// The key each of `ids` shows in the menu, as GPUI keystroke text
/// (`"cmd-shift-p"`): its first binding of a single keystroke whose `when`
/// is empty or holds while the editor has focus. Chords cannot show in a
/// macOS menu, so IDs bound only to chords have no key.
pub fn menu_key_bindings<'a>(keymap: &Keymap, ids: &[&'a str]) -> Vec<(&'a str, String)> {
    let mut editor = WhenContext::default();
    editor.set("editorFocus", ContextValue::Bool(true));
    editor.set("editorTextFocus", ContextValue::Bool(true));
    ids.iter()
        .filter_map(|&id| {
            let binding = keymap
                .bindings_for(id)
                .find(|b| b.keys.len() == 1 && b.when.as_ref().is_none_or(|w| w.eval(&editor)))?;
            Some((id, gpui_keystroke(&binding.keys[0])))
        })
        .collect()
}

/// `keystroke` in GPUI's syntax. The keymap files name keys as GPUI does
/// (`escape`, `enter`, `\\`, `=`, `up`, `f12`), except for the aliases
/// `esc` and `return`.
fn gpui_keystroke(keystroke: &Keystroke) -> String {
    let mut text = String::new();
    for (held, modifier) in [
        (keystroke.ctrl, "ctrl-"),
        (keystroke.alt, "alt-"),
        (keystroke.cmd, "cmd-"),
        (keystroke.shift, "shift-"),
    ] {
        if held {
            text.push_str(modifier);
        }
    }
    text.push_str(match keystroke.key.as_str() {
        "esc" => "escape",
        "return" => "enter",
        key => key,
    });
    text
}

/// Every command ID the menu bar can show, without repeats.
fn menu_command_ids() -> Vec<&'static str> {
    fn collect(entries: &'static [Entry], ids: &mut Vec<&'static str>) {
        for entry in entries {
            match entry {
                Entry::Command { id, .. } => ids.push(id),
                Entry::Submenu { entries, .. } => collect(entries, ids),
                Entry::Only(_, entry) => collect(std::slice::from_ref(*entry), ids),
                Entry::RecentSubmenu => ids.extend([OPEN_RECENT, CLEAR_RECENT]),
                Entry::Separator | Entry::Services | Entry::App(_) => {}
            }
        }
    }
    let mut ids = Vec::new();
    for spec in MENUS {
        collect(spec.entries, &mut ids);
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Marks that [`install`] ran, so [`refresh`] has a menu bar to rebuild.
struct Installed;

impl gpui::Global for Installed {}

/// Sets up the menu bar: the listeners of its actions, rebuilding it when
/// the keymap changes, and the menus themselves.
pub fn install(cx: &mut App) {
    // A menu item's action is dispatched in its window, which cannot be
    // updated again until that returns: the listeners defer.
    cx.on_action(|action: &RunCommand, cx| {
        let id = action.id.clone();
        cx.defer(move |cx| run_command(&id, cx));
    });
    cx.on_action(|action: &OpenRecentEntry, cx| {
        let entry = action.entry.clone();
        cx.defer(move |cx| open_recent_entry(entry, cx));
    });
    cx.on_action(|_: &Quit, cx| crate::workbench::quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                window
                    .update(cx, |_, window, _| window.minimize_window())
                    .ok();
            }
        })
    });
    cx.on_action(|_: &Zoom, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                window.update(cx, |_, window, _| window.zoom_window()).ok();
            }
        })
    });
    cx.set_global(Installed);
    let mut seen = keymap_generation(cx);
    cx.observe_global::<ConfigState>(move |cx| {
        let generation = keymap_generation(cx);
        if generation != seen {
            seen = generation;
            refresh(cx);
        }
    })
    .detach();
    refresh(cx);
}

fn keymap_generation(cx: &App) -> u64 {
    cx.try_global::<ConfigState>()
        .map_or(0, ConfigState::keymap_generation)
}

/// Rebuilds the menu bar: after the keymap or the recent list changed,
/// and when the window's root changed. Binds the menu's keys now; the
/// menus follow once the recent list is read in the background. Does
/// nothing before [`install`].
pub(crate) fn refresh(cx: &mut App) {
    if !cx.has_global::<Installed>() {
        return;
    }
    bind_keys(cx);
    let data_dir = cx
        .try_global::<RecentStore>()
        .and_then(|store| store.data_dir.clone());
    cx.spawn(async move |cx| {
        let recent = match data_dir {
            Some(dir) => {
                cx.background_spawn(
                    async move { recent::load(&recent::recent_path(&dir)).existing() },
                )
                .await
            }
            None => Vec::new(),
        };
        cx.update(|cx| set_menus(&recent, cx));
    })
    .detach();
}

/// GPUI's keymap: the app's fixed keys (⌘Q, ⌘H, ⌥⌘H, ⌘M), then each menu
/// command's key from the Lucerna keymap in [`MENU_CONTEXT`], for both
/// [`RunCommand`] and [`DisabledCommand`], so the menu shows it without
/// running it a second time.
fn bind_keys(cx: &mut App) {
    cx.clear_key_bindings();
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    let Some(state) = cx.try_global::<AppState>() else {
        return;
    };
    let keys = menu_key_bindings(&state.keymap, &menu_command_ids());
    let context = KeyBindingContextPredicate::parse(MENU_CONTEXT)
        .ok()
        .map(Rc::new);
    let mut bindings = Vec::new();
    for (id, keystroke) in keys {
        let actions: [Box<dyn Action>; 2] = [
            Box::new(RunCommand { id: id.into() }),
            Box::new(DisabledCommand { id: id.into() }),
        ];
        for action in actions {
            match KeyBinding::load(&keystroke, action, context.clone(), false, None, cx) {
                Ok(binding) => bindings.push(binding),
                Err(err) => tracing::warn!(%err, id, "cannot show the key in the menu"),
            }
        }
    }
    cx.bind_keys(bindings);
}

fn set_menus(recent: &[RecentEntry], cx: &mut App) {
    let commands = Workbench::command_ids();
    let platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        Platform::Other
    };
    let active = active_window(cx);
    let opened = active
        .and_then(|id| WindowHandle::<Workbench>::new(id).read(cx).ok())
        .map(Workbench::opened_paths);
    let onboarding =
        active.is_some_and(|id| WindowHandle::<OnboardingPage>::new(id).read(cx).is_ok());
    let window = match (&opened, onboarding) {
        (Some(_), _) => WindowKind::Workbench,
        (None, true) => WindowKind::Onboarding,
        (None, false) => WindowKind::Welcome,
    };
    // Choosing the window's own folder would only reopen it.
    let opened = opened.unwrap_or_default();
    let recent: Vec<RecentEntry> = recent
        .iter()
        .filter(|entry| !opened.contains(&entry.path))
        .cloned()
        .collect();
    cx.set_menus(build_menus(&BuildInput {
        commands: &commands,
        platform,
        recent: &recent,
        window,
    }));
}

/// The window menu items act on. A window that started as Welcome keeps
/// its handle type after it shows a workbench, so callers name the root
/// type with `WindowHandle::new` rather than `downcast`.
fn active_window(cx: &App) -> Option<WindowId> {
    cx.active_window()
        .or_else(|| cx.windows().first().copied())
        .map(|window| window.window_id())
}

/// Whether the menu may run `id` in `context`: where its key would. A
/// command whose every binding has a `when` that does not hold is left
/// alone, so Undo with the find box focused does not undo the document.
/// A command with no binding, or one with no `when`, always runs.
fn menu_may_run(keymap: &Keymap, id: &str, context: &WhenContext) -> bool {
    let mut bindings = keymap.bindings_for(id).peekable();
    if bindings.peek().is_none() {
        return true;
    }
    bindings.any(|b| b.when.as_ref().is_none_or(|w| w.eval(context)))
}

/// Runs `id` in the active window's workbench, Welcome screen or Get
/// Started page; with no window, nothing. In a workbench, only where the
/// command's key would run it ([`menu_may_run`]): AppKit fires a menu item
/// for a key the workbench let through.
fn run_command(id: &str, cx: &mut App) {
    let Some(window) = active_window(cx) else {
        return;
    };
    let ran = WindowHandle::<Workbench>::new(window).update(cx, |wb, window, cx| {
        let context = wb.context_keys(window, cx);
        if menu_may_run(&cx.global::<AppState>().keymap, id, &context) {
            wb.run_command(id, None, window, cx);
        }
    });
    if ran.is_err()
        && WindowHandle::<WelcomeView>::new(window)
            .update(cx, |view, window, cx| {
                view.run_command(id, window, cx);
            })
            .is_err()
    {
        WindowHandle::<OnboardingPage>::new(window)
            .update(cx, |page, window, cx| {
                page.run_command(id, window, cx);
            })
            .ok();
    }
}

fn open_recent_entry(entry: RecentEntry, cx: &mut App) {
    let Some(window) = active_window(cx) else {
        return;
    };
    let opened = WindowHandle::<Workbench>::new(window).update(cx, |wb, window, cx| {
        wb.open_recent_entry(entry.clone(), false, window, cx);
    });
    if opened.is_err() {
        WindowHandle::<WelcomeView>::new(window)
            .update(cx, |view, _, cx| view.open_entry(entry, false, cx))
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::recent::RecentKind;

    fn input<'a>(
        commands: &'a [&'a str],
        platform: Platform,
        recent: &'a [RecentEntry],
        window: WindowKind,
    ) -> BuildInput<'a> {
        BuildInput {
            commands,
            platform,
            recent,
            window,
        }
    }

    fn mac(commands: &[&str]) -> Vec<Menu> {
        build_menus(&input(commands, Platform::Mac, &[], WindowKind::Workbench))
    }

    fn menu<'a>(menus: &'a [Menu], name: &str) -> &'a Menu {
        menus
            .iter()
            .find(|m| m.name.as_ref() == name)
            .unwrap_or_else(|| panic!("no {name} menu"))
    }

    fn submenu<'a>(menu: &'a Menu, name: &str) -> &'a Menu {
        menu.items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(m) if m.name.as_ref() == name => Some(m),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {name} submenu"))
    }

    /// Each item's title; "—" for a separator.
    fn titles(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => "—".to_string(),
                MenuItem::Submenu(m) => m.name.to_string(),
                MenuItem::SystemMenu(m) => m.name.to_string(),
                MenuItem::Action { name, .. } => name.to_string(),
            })
            .collect()
    }

    fn item<'a>(menu: &'a Menu, title: &str) -> &'a MenuItem {
        menu.items
            .iter()
            .find(|item| match item {
                MenuItem::Action { name, .. } => name.as_ref() == title,
                MenuItem::Submenu(m) => m.name.as_ref() == title,
                _ => false,
            })
            .unwrap_or_else(|| panic!("no {title} item"))
    }

    fn registered_and(extra: &[&'static str]) -> Vec<&'static str> {
        let mut ids = Workbench::command_ids();
        ids.extend(extra);
        ids
    }

    #[test]
    fn unregistered_commands_are_hidden() {
        let with = registered_and(&["editor.action.commentLine"]);
        let edit = titles(menu(&mac(&with), "Edit"));
        assert_eq!(edit.last().map(String::as_str), Some("Toggle Line Comment"));

        let without: Vec<&str> = with
            .iter()
            .copied()
            .filter(|id| *id != "editor.action.commentLine")
            .collect();
        let edit = titles(menu(&mac(&without), "Edit"));
        assert!(!edit.iter().any(|t| t == "Toggle Line Comment"));
        assert_ne!(edit.last().map(String::as_str), Some("—"));
    }

    #[test]
    fn help_lists_third_party_licenses() {
        let menus = mac(&Workbench::command_ids());
        assert!(titles(menu(&menus, "Help")).contains(&"Third-Party Licenses".to_string()));
    }

    #[test]
    fn separators_collapse() {
        // With only Select All registered, the Selection menu's other groups
        // are empty and their separators must not be left behind.
        let menus = mac(&["editor.action.selectAll"]);
        assert_eq!(titles(menu(&menus, "Selection")), ["Select All"]);
        // With every command, the groups come back with one separator each.
        let menus = mac(&Workbench::command_ids());
        let selection = titles(menu(&menus, "Selection"));
        assert!(selection.len() > 1, "{selection:?}");
        assert!(selection.windows(2).all(|w| !(w[0] == "—" && w[1] == "—")));
        assert_ne!(selection.last().map(String::as_str), Some("—"));
    }

    #[test]
    fn mac_has_app_menu_first() {
        let menus = mac(&Workbench::command_ids());
        assert_eq!(menus[0].name.as_ref(), "Lucerna");
        let app = titles(&menus[0]);
        assert!(app.iter().any(|t| t == "Services"), "{app:?}");
        assert!(app.iter().any(|t| t == "Hide Others"), "{app:?}");
        assert_eq!(app.first().map(String::as_str), Some("About Lucerna"));
        let file = titles(menu(&menus, "File"));
        assert_eq!(file.first().map(String::as_str), Some("Open…"));
    }

    #[test]
    fn other_platforms_move_app_items() {
        let commands = Workbench::command_ids();
        let menus = build_menus(&input(
            &commands,
            Platform::Other,
            &[],
            WindowKind::Workbench,
        ));
        assert!(menus.iter().all(|m| m.name.as_ref() != "Lucerna"));
        let help = titles(menu(&menus, "Help"));
        assert_eq!(help.last().map(String::as_str), Some("About Lucerna"));
        let file = menu(&menus, "File");
        assert_eq!(titles(file).last().map(String::as_str), Some("Exit"));
        assert_eq!(titles(file).first().map(String::as_str), Some("Open File…"));
        assert!(titles(submenu(file, "Preferences")).contains(&"Color Theme…".to_string()));
        for menu in &menus {
            let titles = titles(menu);
            assert!(!titles.iter().any(|t| t == "Services"), "{titles:?}");
            assert!(!titles.iter().any(|t| t.starts_with("Hide")), "{titles:?}");
        }
    }

    #[test]
    fn welcome_disables_workbench_items() {
        let commands = Workbench::command_ids();
        let menus = build_menus(&input(&commands, Platform::Mac, &[], WindowKind::Welcome));
        let file = menu(&menus, "File");
        assert!(item(file, "Save").is_disabled());
        match item(file, "Save") {
            MenuItem::Action { action, .. } => assert!(action.partial_eq(&DisabledCommand {
                id: "workbench.action.files.save".into()
            })),
            _ => panic!("Save is an action"),
        }
        match item(file, "Open…") {
            MenuItem::Action { action, .. } => assert!(action.partial_eq(&RunCommand {
                id: "workbench.action.files.openFileFolder".into()
            })),
            _ => panic!("Open… is an action"),
        }
        assert!(!item(file, "Open…").is_disabled());
        assert!(!item(file, "Open Recent").is_disabled());
        assert!(!item(submenu(file, "Open Recent"), "More…").is_disabled());
        let app = menu(&menus, "Lucerna");
        assert!(!item(app, "About Lucerna").is_disabled());
        assert!(!item(app, "Quit Lucerna").is_disabled());
        assert!(item(app, "Settings").is_disabled());
        assert!(!item(menu(&menus, "Window"), "Minimize").is_disabled());

        let menus = build_menus(&input(&commands, Platform::Mac, &[], WindowKind::Workbench));
        assert!(!item(menu(&menus, "File"), "Save").is_disabled());
    }

    #[test]
    fn help_starts_with_get_started() {
        let commands = registered_and(&["lucerna.openOnboarding"]);
        let help = titles(menu(&mac(&commands), "Help"));
        assert_eq!(&help[..3], ["Get Started", "—", "Show All Commands"]);
        // Unregistered, it is left out with its separator.
        let help = titles(menu(&mac(&Workbench::command_ids()), "Help"));
        assert_eq!(help.first().map(String::as_str), Some("Show All Commands"));
    }

    #[test]
    fn welcome_enables_get_started() {
        let commands = registered_and(&["lucerna.openOnboarding"]);
        let menus = build_menus(&input(&commands, Platform::Mac, &[], WindowKind::Welcome));
        assert!(!item(menu(&menus, "Help"), "Get Started").is_disabled());
    }

    #[test]
    fn onboarding_enables_only_its_commands() {
        let commands = registered_and(&["lucerna.openOnboarding"]);
        let recent = [RecentEntry {
            path: PathBuf::from("/p"),
            kind: RecentKind::Folder,
        }];
        let menus = build_menus(&input(
            &commands,
            Platform::Mac,
            &recent,
            WindowKind::Onboarding,
        ));
        let file = menu(&menus, "File");
        assert!(item(file, "Save").is_disabled());
        assert!(item(file, "Open…").is_disabled());
        // The page cannot open a recent entry.
        let open_recent = submenu(file, "Open Recent");
        assert!(!titles(open_recent).contains(&"/p".to_string()));
        assert!(item(open_recent, "More…").is_disabled());
        assert!(!item(menu(&menus, "Help"), "Get Started").is_disabled());
        let app = menu(&menus, "Lucerna");
        assert!(!item(app, "About Lucerna").is_disabled());
        assert!(!item(app, "Quit Lucerna").is_disabled());
        assert!(!item(submenu(app, "Settings"), "Open Settings (JSON)").is_disabled());
    }

    #[test]
    fn open_recent_submenu() {
        let recent: Vec<RecentEntry> = (0..12)
            .map(|i| RecentEntry {
                path: PathBuf::from(format!("/projects/p{i}")),
                kind: RecentKind::Folder,
            })
            .collect();
        let commands = Workbench::command_ids();
        let menus = build_menus(&input(
            &commands,
            Platform::Mac,
            &recent,
            WindowKind::Workbench,
        ));
        let open_recent = submenu(menu(&menus, "File"), "Open Recent");
        let titles = titles(open_recent);
        assert_eq!(titles.len(), 13, "{titles:?}");
        assert_eq!(titles[0], "/projects/p0");
        assert_eq!(titles[9], "/projects/p9");
        assert_eq!(&titles[10..], ["—", "More…", "Clear Recently Opened…"]);
        match &open_recent.items[0] {
            MenuItem::Action { action, .. } => assert!(action.partial_eq(&OpenRecentEntry {
                entry: recent[0].clone()
            })),
            _ => panic!("a recent entry is an action"),
        }
    }

    #[test]
    fn edit_uses_os_actions() {
        let menus = mac(&Workbench::command_ids());
        let edit = menu(&menus, "Edit");
        match item(edit, "Cut") {
            MenuItem::Action { os_action, .. } => assert!(*os_action == Some(OsAction::Cut)),
            _ => panic!("Cut is an action"),
        }
        menu(&menus, "Help");
    }

    #[test]
    fn menu_keys_follow_keymap_and_skip_chords() {
        let ids = ["workbench.action.quickOpen", "workbench.action.closeFolder"];
        let keymap = |user: Option<&str>| {
            let (keymap, error) = Keymap::load(Keymap::default_bindings(), user);
            assert!(error.is_none());
            keymap
        };
        assert_eq!(
            menu_key_bindings(&keymap(None), &ids),
            [("workbench.action.quickOpen", "cmd-p".to_string())]
        );
        let remapped = keymap(Some(
            r#"[{ "key": "cmd+shift+o", "command": "workbench.action.quickOpen" }]"#,
        ));
        assert_eq!(
            menu_key_bindings(&remapped, &ids),
            [("workbench.action.quickOpen", "cmd-shift-o".to_string())]
        );
        let removed = keymap(Some(
            r#"[{ "key": "cmd+p", "command": "-workbench.action.quickOpen" }]"#,
        ));
        assert_eq!(menu_key_bindings(&removed, &ids), []);
    }

    #[test]
    fn menu_runs_commands_where_their_key_would() {
        let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
        assert!(error.is_none());
        let mut text_input = WhenContext::default();
        text_input.set("editorFocus", ContextValue::Bool(true));
        text_input.set("editorTextFocus", ContextValue::Bool(false));
        let mut editor = text_input.clone();
        editor.set("editorTextFocus", ContextValue::Bool(true));
        // `when` is `editorTextFocus && !editorReadonly`.
        assert!(!menu_may_run(&keymap, "undo", &text_input));
        assert!(menu_may_run(&keymap, "undo", &editor));
        // ⌘B has no `when`; Clear Recently Opened has no key.
        let none = WhenContext::default();
        assert!(menu_may_run(
            &keymap,
            "workbench.action.toggleSidebarVisibility",
            &none
        ));
        assert!(menu_may_run(&keymap, CLEAR_RECENT, &none));
    }

    #[test]
    fn keystrokes_use_gpui_key_names() {
        let text = |s: &str| gpui_keystroke(&lucerna_settings::parse_keys(s).unwrap()[0]);
        assert_eq!(text("ctrl+shift+-"), "ctrl-shift--");
        assert_eq!(text("cmd+\\"), "cmd-\\");
        assert_eq!(text("alt+cmd+up"), "alt-cmd-up");
        assert_eq!(text("shift+f12"), "shift-f12");
        assert_eq!(text("ctrl+esc"), "ctrl-escape");
        for s in [
            "ctrl-shift--",
            "cmd-\\",
            "cmd-=",
            "alt-cmd-up",
            "shift-f12",
            "cmd-enter",
        ] {
            assert!(gpui::Keystroke::parse(s).is_ok(), "{s}");
        }
    }
}
