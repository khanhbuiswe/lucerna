//! Links in a terminal view (spec §6.7): with ⌘ (ctrl off macOS) held, the
//! link under the mouse is underlined and ⌘+click opens it.
//!
//! Hover scans the last painted frame's cells with `link_at` (no terminal
//! lock), at most every [`HOVER_THROTTLE`] and only after the mouse moved
//! [`HOVER_MIN_MOVE`]. A URL shows at once; a path shows only once
//! `resolve_path` found it on a background thread. Resolutions are cached for
//! [`RESOLVE_CACHE_TTL`], as VS Code's terminal link cache does, so hovering
//! does not stat the same text again and again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, IntoElement, Modifiers, ParentElement, Pixels, Point, Styled, Task,
    div, point, prelude::*, px,
};
use lucerna_settings::SettingsView;
use lucerna_terminal::{
    LinkKind, ResolveCtx, Resolved, ViewStamp, foreground_process, link_at, resolve_path,
};

use super::settings::home_dir;
use super::view::{TerminalView, TerminalViewEvent};
use crate::AppState;
use crate::layout::color;
use crate::ui::{self, GRID};

/// Hover scans at most this often (spec §6.7).
const HOVER_THROTTLE: Duration = Duration::from_millis(100);
/// A move shorter than this since the last scan does not scan again.
const HOVER_MIN_MOVE: f32 = 5.;
/// Time `link_at` may spend on one hover scan (spec §6.7).
const SCAN_BUDGET: Duration = Duration::from_millis(1);
/// A click is rare and asks for the link outright, so it may take longer.
const CLICK_SCAN_BUDGET: Duration = Duration::from_millis(10);
/// How long a path resolution is reused.
const RESOLVE_CACHE_TTL: Duration = Duration::from_secs(10);
/// The foreground process's cwd is asked for at most this often.
const CWD_POLL: Duration = Duration::from_millis(500);
/// `terminal.integrated.allowedLinkSchemes` when unset (VS Code's default
/// trimmed to the spec's list).
const DEFAULT_LINK_SCHEMES: [&str; 4] = ["http", "https", "mailto", "file"];
/// Thickness of the hover underline.
const UNDERLINE: f32 = 1.;

/// What a link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Opened in the browser.
    Url(String),
    /// Opened in the editor, at the 1-based line and column when given.
    File {
        path: PathBuf,
        line: Option<u32>,
        col: Option<u32>,
    },
    /// Revealed in the Explorer when inside the workspace.
    Dir(PathBuf),
}

impl LinkTarget {
    fn from_resolved(r: Resolved) -> Self {
        match r {
            Resolved::File { path, line, col } => LinkTarget::File { path, line, col },
            Resolved::Dir(path) => LinkTarget::Dir(path),
        }
    }

    /// The target as the tooltip names it.
    fn label(&self) -> String {
        match self {
            LinkTarget::Url(url) => url.clone(),
            LinkTarget::File { path, line, col } => {
                let mut s = path.display().to_string();
                if let Some(line) = line {
                    s.push_str(&format!(":{line}"));
                    if let Some(col) = col {
                        s.push_str(&format!(":{col}"));
                    }
                }
                s
            }
            LinkTarget::Dir(path) => path.display().to_string(),
        }
    }
}

/// The link shown under the mouse: its cells on the painted screen and
/// what it opens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShownLink {
    start: (u16, u16),
    end: (u16, u16),
    stamp: ViewStamp,
    target: LinkTarget,
}

impl ShownLink {
    fn covers(&self, row: u16, col: u16) -> bool {
        self.start <= (row, col) && (row, col) <= self.end
    }
}

/// A link `link_at` found, before a path is resolved.
struct Candidate {
    start: (u16, u16),
    end: (u16, u16),
    stamp: ViewStamp,
    kind: CandidateKind,
}

enum CandidateKind {
    Url(String),
    Path {
        raw: String,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// Text, line, column and the foreground cwd a resolution was made for.
type CacheKey = (String, Option<u32>, Option<u32>, Option<PathBuf>);

/// A view's link state between events.
#[derive(Default)]
pub(crate) struct LinkState {
    shown: Option<ShownLink>,
    /// When and where the last scan ran, for the throttle.
    last_scan: Option<(Instant, Point<Pixels>)>,
    /// The latest position a throttled move left for the trailing scan.
    pending_pos: Option<Point<Pixels>>,
    trailing: Option<Task<()>>,
    /// The hover resolution in flight; dropping it cancels it.
    resolving: Option<Task<()>>,
    /// The resolution of a ⌘+clicked path. Kept apart from `resolving`, so
    /// letting go of ⌘ or moving on right after the click does not drop it.
    opening: Option<Task<()>>,
    /// Numbers resolutions, so only the latest one of each kind is applied.
    request: u64,
    /// The `request` number of the click in `opening`.
    open_request: u64,
    cache: HashMap<CacheKey, (Instant, Option<Resolved>)>,
    /// The foreground process's cwd, when known: from the view's process
    /// poll, or polled here at most every [`CWD_POLL`] while links are
    /// looked up.
    pub(crate) cwd_now: Option<PathBuf>,
    cwd_polled: Option<Instant>,
    /// How many times `link_at` ran, for tests.
    scans: u64,
    /// Replaces [`SCAN_BUDGET`] in tests, where a loaded machine running a
    /// debug build can miss 1 ms.
    budget: Option<Duration>,
}

impl LinkState {
    /// The view's process poll read `cwd` at `at`: links use it, and need
    /// not poll again before [`CWD_POLL`] passed.
    pub(crate) fn note_polled_cwd(&mut self, cwd: PathBuf, at: Instant) {
        self.cwd_now = Some(cwd);
        self.cwd_polled = Some(at);
    }
}

/// What links read from settings.
struct LinkSettings {
    /// `terminal.integrated.enableFileLinks` is not `"off"`.
    file_links: bool,
    /// `terminal.integrated.allowedLinkSchemes`, lowercase.
    schemes: Vec<String>,
}

fn link_settings(settings: SettingsView<'_>) -> LinkSettings {
    let file_links = settings
        .get::<serde_json::Value>("terminal.integrated.enableFileLinks", None)
        .is_none_or(|v| match v {
            serde_json::Value::String(s) => s != "off",
            serde_json::Value::Bool(b) => b,
            _ => true,
        });
    let schemes = settings
        .get::<Vec<String>>("terminal.integrated.allowedLinkSchemes", None)
        .unwrap_or_else(|| DEFAULT_LINK_SCHEMES.iter().map(|s| s.to_string()).collect())
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    LinkSettings {
        file_links,
        schemes,
    }
}

/// Whether `m` holds the link modifier: ⌘ on macOS, ctrl elsewhere.
pub(crate) fn link_modifier(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.platform
    } else {
        m.control
    }
}

/// The hint after the target in the tooltip.
fn open_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+click to open"
    } else {
        "Ctrl+click to open"
    }
}

/// The scheme of `url` (before `:`), lowercase.
fn scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let valid = !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// The local path a `file://` URL names, if it is one.
pub(crate) fn file_url_to_path(url: &str) -> Option<PathBuf> {
    if scheme(url)? != "file" {
        return None;
    }
    url::Url::parse(url).ok()?.to_file_path().ok()
}

/// The user's home folder, for `~/` paths.
impl TerminalView {
    /// A link is underlined.
    pub(crate) fn link_shown(&self) -> bool {
        self.links.shown.is_some()
    }

    /// A link is underlined or being looked up.
    pub(crate) fn link_active(&self) -> bool {
        self.links.shown.is_some()
            || self.links.resolving.is_some()
            || self.links.trailing.is_some()
    }

    /// How many times hover scanned for a link.
    #[doc(hidden)]
    pub fn link_scans(&self) -> u64 {
        self.links.scans
    }

    /// Lets hover scans take `budget` instead of 1 ms.
    #[doc(hidden)]
    pub fn set_link_scan_budget(&mut self, budget: Duration) {
        self.links.budget = Some(budget);
    }

    /// Sets the foreground process's cwd that relative paths resolve
    /// against first.
    #[doc(hidden)]
    pub fn set_cwd_now(&mut self, cwd: Option<PathBuf>) {
        self.links.cwd_now = cwd;
    }

    /// A new frame was laid out: a link drawn for another screen goes.
    pub(crate) fn links_frame_changed(&mut self, stamp: ViewStamp) {
        if self.links.shown.as_ref().is_some_and(|s| s.stamp != stamp) {
            self.links.shown = None;
        }
    }

    fn set_shown(&mut self, shown: Option<ShownLink>, cx: &mut Context<Self>) {
        if self.links.shown != shown {
            self.links.shown = shown;
            cx.notify();
        }
    }

    /// The modifier was let go or the mouse left: hides the link and stops
    /// looking.
    pub(crate) fn release_link(&mut self, cx: &mut Context<Self>) {
        self.links.resolving = None;
        self.links.trailing = None;
        self.links.pending_pos = None;
        self.links.last_scan = None;
        self.set_shown(None, cx);
    }

    /// The modifiers changed (heard at the workbench root, focused or
    /// not): the link modifier going down over the grid looks for a link
    /// at once; going up hides the link and drops a pending trailing scan.
    /// `visible`: the panel shows this view.
    pub(crate) fn link_modifiers_changed(
        &mut self,
        m: &Modifiers,
        pos: Point<Pixels>,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if !link_modifier(m) {
            if self.link_active() {
                self.release_link(cx);
            }
        } else if visible && !self.mouse_pressed() {
            self.scan_link(pos, cx);
        }
    }

    /// The mouse moved over the grid with no button held.
    pub(crate) fn link_mouse_move(
        &mut self,
        pos: Point<Pixels>,
        m: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        if !link_modifier(m) {
            if self.link_active() {
                self.release_link(cx);
            }
            return;
        }
        let now = cx.background_executor().now();
        if let Some((at, from)) = self.links.last_scan {
            let moved = pos - from;
            if f32::from(moved.x).hypot(f32::from(moved.y)) < HOVER_MIN_MOVE {
                return;
            }
            let due = at + HOVER_THROTTLE;
            if now < due {
                self.links.pending_pos = Some(pos);
                if self.links.trailing.is_none() {
                    let delay = due - now;
                    self.links.trailing = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await;
                        let _ = this.update(cx, |view, cx| {
                            view.links.trailing = None;
                            if let Some(pos) = view.links.pending_pos.take() {
                                view.scan_link(pos, cx);
                            }
                        });
                    }));
                }
                return;
            }
        }
        self.scan_link(pos, cx);
    }

    /// Looks for a link at `pos` and shows it (a path once resolved).
    fn scan_link(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        self.links.last_scan = Some((cx.background_executor().now(), pos));
        self.links.pending_pos = None;
        let budget = self.links.budget.unwrap_or(SCAN_BUDGET);
        match self.candidate_at(pos, budget) {
            Some(candidate) => {
                self.take_candidate(candidate, false, cx);
            }
            None => {
                self.links.resolving = None;
                self.set_shown(None, cx);
            }
        }
    }

    /// ⌘+click at `pos`: opens the link there; whether there was one, so
    /// the press neither selects nor reaches the app.
    pub(crate) fn link_click(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.last_frame.clone() else {
            return false;
        };
        if !frame.geometry.grid_bounds().contains(&pos) {
            return false;
        }
        let (row, col, _) = frame.geometry.cell_at(pos);
        if let Some(shown) = self
            .links
            .shown
            .clone()
            .filter(|s| s.stamp == frame.stamp && s.covers(row, col))
        {
            self.open_link(shown.target, cx);
            return true;
        }
        let budget = self.links.budget.unwrap_or(CLICK_SCAN_BUDGET);
        match self.candidate_at(pos, budget) {
            Some(candidate) => self.take_candidate(candidate, true, cx),
            None => false,
        }
    }

    /// The link `link_at` finds at `pos` on the painted screen.
    fn candidate_at(&mut self, pos: Point<Pixels>, budget: Duration) -> Option<Candidate> {
        self.links.scans += 1;
        let frame = self.last_frame.clone()?;
        if !frame.geometry.grid_bounds().contains(&pos) {
            return None;
        }
        let (row, col, _) = frame.geometry.cell_at(pos);
        let link = link_at(&frame.cells, &frame.wrapped, row, col, budget)?;
        let kind = match link.kind {
            LinkKind::Url(url) => match file_url_to_path(&url) {
                Some(path) => CandidateKind::Path {
                    raw: path.to_string_lossy().into_owned(),
                    line: None,
                    col: None,
                },
                None => CandidateKind::Url(url),
            },
            LinkKind::Path { path, line, col } => CandidateKind::Path {
                raw: path,
                line,
                col,
            },
        };
        Some(Candidate {
            start: link.start,
            end: link.end,
            stamp: frame.stamp,
            kind,
        })
    }

    /// Shows `c` (or opens it, with `open`); a path is resolved first, from
    /// the cache or on a background thread. Whether `c` is, or may turn out
    /// to be, a link.
    fn take_candidate(&mut self, c: Candidate, open: bool, cx: &mut Context<Self>) -> bool {
        let settings = link_settings(self.settings.read(cx).view(cx));
        let shown = |target| ShownLink {
            start: c.start,
            end: c.end,
            stamp: c.stamp,
            target,
        };
        match &c.kind {
            CandidateKind::Url(url) => {
                self.links.resolving = None;
                let allowed = scheme(url).is_some_and(|s| settings.schemes.contains(&s));
                if !allowed {
                    self.set_shown(None, cx);
                    return false;
                }
                let target = LinkTarget::Url(url.clone());
                if open {
                    self.open_link(target, cx);
                } else {
                    self.set_shown(Some(shown(target)), cx);
                }
                true
            }
            CandidateKind::Path { raw, line, col } => {
                if !settings.file_links {
                    self.links.resolving = None;
                    self.set_shown(None, cx);
                    return false;
                }
                let key: CacheKey = (raw.clone(), *line, *col, self.links.cwd_now.clone());
                let now = cx.background_executor().now();
                self.links
                    .cache
                    .retain(|_, (at, _)| now.saturating_duration_since(*at) < RESOLVE_CACHE_TTL);
                if let Some((_, cached)) = self.links.cache.get(&key) {
                    self.links.resolving = None;
                    let Some(resolved) = cached.clone() else {
                        self.set_shown(None, cx);
                        return false;
                    };
                    let target = LinkTarget::from_resolved(resolved);
                    if open {
                        self.open_link(target, cx);
                    } else {
                        self.set_shown(Some(shown(target)), cx);
                    }
                    return true;
                }
                // Nothing is drawn until the path is known to exist.
                self.set_shown(None, cx);
                self.resolve(c, open, cx);
                true
            }
        }
    }

    /// Resolves the path of `c` on a background thread (polling the
    /// foreground cwd first when it is due), then shows or opens it.
    fn resolve(&mut self, c: Candidate, open: bool, cx: &mut Context<Self>) {
        let CandidateKind::Path { raw, line, col } = c.kind else {
            return;
        };
        let now = cx.background_executor().now();
        let poll = self
            .links
            .cwd_polled
            .is_none_or(|at| now.saturating_duration_since(at) >= CWD_POLL);
        if poll {
            self.links.cwd_polled = Some(now);
        }
        self.links.request += 1;
        let request = self.links.request;
        let terminal = self.terminal.clone();
        let known_cwd = self.links.cwd_now.clone();
        let cwd_start = self.cwd_start.clone();
        let root = self.root.clone();
        let task = cx.background_spawn({
            let raw = raw.clone();
            async move {
                let polled = if poll {
                    foreground_process(&terminal).and_then(|p| p.cwd)
                } else {
                    None
                };
                let cwd = polled.clone().or(known_cwd);
                let home = home_dir();
                let ctx = ResolveCtx {
                    cwd_now: cwd.as_deref(),
                    cwd_start: &cwd_start,
                    root: &root,
                    home: home.as_deref(),
                };
                let resolved = resolve_path(&raw, line, col, &ctx);
                (polled, cwd, resolved)
            }
        });
        let (start, end, stamp) = (c.start, c.end, c.stamp);
        let slot = cx.spawn(async move |this, cx| {
            let (polled, cwd, resolved) = task.await;
            let _ = this.update(cx, |view, cx| {
                if polled.is_some() {
                    view.links.cwd_now = polled;
                }
                let at = cx.background_executor().now();
                view.links
                    .cache
                    .insert((raw, line, col, cwd), (at, resolved.clone()));
                let target = resolved.map(LinkTarget::from_resolved);
                if open {
                    if view.links.open_request == request {
                        view.links.opening = None;
                        if let Some(target) = target {
                            view.open_link(target, cx);
                        }
                    }
                    return;
                }
                if view.links.request != request {
                    return;
                }
                view.links.resolving = None;
                if let Some(target) = target {
                    let shown = ShownLink {
                        start,
                        end,
                        stamp,
                        target,
                    };
                    view.set_shown(Some(shown), cx);
                }
            });
        });
        if open {
            self.links.open_request = request;
            self.links.opening = Some(slot);
        } else {
            self.links.resolving = Some(slot);
        }
    }

    /// Opens `target`: a URL in the browser, a file or folder through the
    /// workbench.
    fn open_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        match target {
            LinkTarget::Url(url) => cx.open_url(&url),
            target => cx.emit(TerminalViewEvent::OpenLink(target)),
        }
    }

    /// The underline under the shown link and its tooltip, placed over the
    /// view.
    pub(crate) fn link_overlay(&self, cx: &App) -> Vec<AnyElement> {
        let (Some(shown), Some(frame)) = (&self.links.shown, &self.last_frame) else {
            return Vec::new();
        };
        if shown.stamp != frame.stamp {
            return Vec::new();
        }
        let g = &frame.geometry;
        let origin = frame.bounds.origin;
        let ui_colors = &cx.global::<AppState>().theme.ui;
        let mut out = Vec::new();
        let last_col = g.cols.saturating_sub(1);
        for row in shown.start.0..=shown.end.0 {
            let from = if row == shown.start.0 {
                shown.start.1
            } else {
                0
            };
            let to = if row == shown.end.0 {
                shown.end.1.min(last_col)
            } else {
                last_col
            };
            if from > to {
                continue;
            }
            let cells = g.cells(row, from, to - from + 1);
            out.push(
                div()
                    .debug_selector(|| "terminal-link-hover".into())
                    .absolute()
                    .left(cells.left() - origin.x)
                    .top(cells.bottom() - origin.y - px(UNDERLINE))
                    .w(cells.size.width)
                    .h(px(UNDERLINE))
                    .bg(color(ui_colors.text))
                    .into_any_element(),
            );
        }

        // Below the link, or above it near the bottom edge.
        let first = g.cells(shown.start.0, shown.start.1, 1);
        let last = g.cells(shown.end.0, 0, 1);
        let tooltip_h = px(6. * GRID);
        let below = last.bottom() + px(GRID);
        let top = if below + tooltip_h > frame.bounds.bottom() {
            first.top() - px(GRID) - tooltip_h
        } else {
            below
        };
        let at = point(first.left() - origin.x, (top - origin.y).max(px(0.)));
        let muted = color(ui_colors.text_muted);
        out.push(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .max_w(frame.bounds.size.width - at.x)
                .child(
                    ui::tooltip_box(cx)
                        .debug_selector(|| "terminal-link-tooltip".into())
                        .flex()
                        .flex_row()
                        .gap(px(2. * GRID))
                        .overflow_hidden()
                        .child(div().truncate().child(shown.target.label()))
                        .child(div().flex_none().text_color(muted).child(open_hint())),
                )
                .into_any_element(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_become_paths() {
        #[cfg(unix)]
        {
            assert_eq!(
                file_url_to_path("file:///tmp/a%20b.rs"),
                Some(PathBuf::from("/tmp/a b.rs"))
            );
            assert_eq!(
                file_url_to_path("FILE:///etc/hosts"),
                Some(PathBuf::from("/etc/hosts"))
            );
        }
        assert_eq!(file_url_to_path("https://example.com/x"), None);
        // Another machine's file is no local path.
        #[cfg(unix)]
        assert_eq!(file_url_to_path("file://server/share/x"), None);
    }

    #[test]
    fn schemes_are_read_case_insensitively() {
        assert_eq!(scheme("HTTPS://x").as_deref(), Some("https"));
        assert_eq!(scheme("mailto:a@b").as_deref(), Some("mailto"));
        assert_eq!(scheme("ftp://x").as_deref(), Some("ftp"));
        assert_eq!(scheme("no scheme"), None);
        assert_eq!(scheme("://x"), None);
    }

    #[test]
    fn labels_name_the_target() {
        let file = LinkTarget::File {
            path: PathBuf::from("/p/a.rs"),
            line: Some(3),
            col: Some(2),
        };
        assert_eq!(file.label(), "/p/a.rs:3:2");
        assert_eq!(LinkTarget::Dir(PathBuf::from("/p")).label(), "/p");
    }

    #[test]
    fn shown_link_covers_wrapped_rows() {
        let link = ShownLink {
            start: (0, 70),
            end: (1, 5),
            stamp: ViewStamp::default(),
            target: LinkTarget::Url("https://x".into()),
        };
        assert!(link.covers(0, 75));
        assert!(link.covers(1, 0));
        assert!(!link.covers(0, 69));
        assert!(!link.covers(1, 6));
    }

    #[test]
    fn terminal_is_shareable_with_the_background() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<std::sync::Arc<lucerna_terminal::Terminal>>();
    }
}
