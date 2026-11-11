//! ⌘-hover definition link and ⌘-click (spec §7.3).
//!
//! With the platform's secondary modifier held (⌘ on macOS, Ctrl
//! elsewhere) the char under the mouse asks the servers for its
//! definition; an answer shows its origin range (or the word) as a link:
//! underlined, in `link_text.hover`, with a pointing hand. One answer is
//! kept per range, so moving within it sends nothing; moving to another
//! word drops the waiting request, which cancels it. Releasing the
//! modifier or leaving the range hides the link. While the modifier is
//! held the hover does not show, as in Zed.
//!
//! ⌘-click puts the cursor at the click, as a plain click does, then goes
//! to the definition from there (⌥⌘-click: in the group to the right);
//! an empty answer is silent. A click where the last answer found nothing
//! is a plain click.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{Context, Entity, Task, WeakEntity, Window};
use lucerna_text::Point;

use super::{CodeView, word_at};
use crate::Workbench;
use crate::goto::{GotoKind, GotoMode, GotoSource, goto_at};
use crate::language_servers::LanguageServers;

/// The definition answer for a range.
struct Answer {
    range: Range<Point>,
    /// Whether a definition was found: the range is a link.
    linked: bool,
}

/// A request waiting for its answer.
struct Pending {
    point: Point,
    /// The word under `point`: moving within it keeps waiting.
    word: Range<Point>,
    _task: Task<()>,
}

/// A ⌘-click to act on once the code view is no longer being updated.
pub(crate) struct LinkClick {
    point: Point,
    aside: bool,
    workbench: WeakEntity<Workbench>,
}

impl LinkClick {
    /// Goes to the definition at the click, from `view`.
    pub(crate) fn run(self, view: &Entity<CodeView>, window: &mut Window, cx: &mut gpui::App) {
        let Some(wb) = self.workbench.upgrade() else {
            return;
        };
        let mode = if self.aside {
            GotoMode::Aside
        } else {
            GotoMode::Reveal
        };
        wb.update(cx, |wb, cx| {
            goto_at(
                wb,
                view,
                self.point,
                GotoKind::Definition,
                mode,
                GotoSource::Click,
                window,
                cx,
            );
        });
    }
}

/// The link state of one code view.
#[derive(Default)]
pub(crate) struct Link {
    servers: Option<WeakEntity<LanguageServers>>,
    workbench: Option<WeakEntity<Workbench>>,
    /// The char with text under the mouse.
    mouse: Option<Point>,
    /// Whether the secondary modifier is held.
    held: bool,
    /// The last answer, for the current text.
    cached: Option<Answer>,
    pending: Option<Pending>,
    /// The range shown as a link.
    shown: Option<Range<Point>>,
}

impl Link {
    pub(crate) fn set_sources(
        &mut self,
        workbench: WeakEntity<Workbench>,
        servers: WeakEntity<LanguageServers>,
    ) {
        self.workbench = Some(workbench);
        self.servers = Some(servers);
    }

    /// Whether the secondary modifier is held, as last seen.
    pub(crate) fn held(&self) -> bool {
        self.held
    }

    /// Whether the last answer says `p` links nowhere.
    fn known_unlinked(&self, p: Point) -> bool {
        self.cached
            .as_ref()
            .is_some_and(|a| !a.linked && char_in(&a.range, p))
    }
}

impl CodeView {
    /// The range shown as a definition link, while ⌘ is held over it.
    pub fn link_range(&self) -> Option<Range<Point>> {
        self.link.shown.clone()
    }

    pub(crate) fn set_link_sources(
        &mut self,
        workbench: WeakEntity<Workbench>,
        servers: WeakEntity<LanguageServers>,
    ) {
        self.link.set_sources(workbench, servers);
    }

    /// The mouse is over the char `point` (`None`: no char), with the
    /// secondary modifier `held` or not.
    pub(crate) fn link_mouse_moved(
        &mut self,
        point: Option<Point>,
        held: bool,
        cx: &mut Context<Self>,
    ) {
        self.link.mouse = point;
        self.link.held = held;
        self.update_link(cx);
    }

    /// The secondary modifier was pressed or released.
    pub(crate) fn link_modifiers_changed(&mut self, held: bool, cx: &mut Context<Self>) {
        if self.link.held == held {
            return;
        }
        self.link.held = held;
        if held && self.link.mouse.is_some() {
            self.hover.hide(cx);
        }
        self.update_link(cx);
    }

    /// Forgets answers and the link: the text changed.
    pub(crate) fn reset_link(&mut self, cx: &mut Context<Self>) {
        self.link.cached = None;
        self.link.pending = None;
        self.set_link_shown(None, cx);
    }

    /// A click with the secondary modifier on the char `p`: moves the
    /// cursor there and returns the lookup to run, unless the last answer
    /// says `p` links nowhere.
    pub(crate) fn link_click(
        &mut self,
        p: Point,
        aside: bool,
        cx: &mut Context<Self>,
    ) -> Option<LinkClick> {
        if self.link.known_unlinked(p) {
            return None;
        }
        let workbench = self.link.workbench.clone()?;
        self.set_link_shown(None, cx);
        Some(LinkClick {
            point: p,
            aside,
            workbench,
        })
    }

    fn update_link(&mut self, cx: &mut Context<Self>) {
        let (true, Some(p)) = (self.link.held, self.link.mouse) else {
            self.link.pending = None;
            self.set_link_shown(None, cx);
            return;
        };
        if let Some(answer) = self.link.cached.as_ref().filter(|a| char_in(&a.range, p)) {
            let shown = answer.linked.then(|| answer.range.clone());
            self.link.pending = None;
            self.set_link_shown(shown, cx);
            return;
        }
        self.set_link_shown(None, cx);
        if self
            .link
            .pending
            .as_ref()
            .is_some_and(|pending| char_in(&pending.word, p))
        {
            return;
        }
        let word = word_at(&self.snapshot, p);
        let task = self.request_link(p, cx);
        self.link.pending = Some(Pending {
            point: p,
            word,
            _task: task,
        });
    }

    fn request_link(&mut self, p: Point, cx: &mut Context<Self>) -> Task<()> {
        let servers = self.link.servers.as_ref().and_then(WeakEntity::upgrade);
        let path: Arc<Path> = self.path.clone();
        let request = match servers {
            Some(servers) => servers.update(cx, |s, cx| s.definition_link(&path, p, cx)),
            None => Task::ready(None),
        };
        cx.spawn(async move |view, cx| {
            let range = request.await;
            view.update(cx, |v, cx| v.finish_link(p, range, cx)).ok();
        })
    }

    fn finish_link(&mut self, p: Point, range: Option<Range<Point>>, cx: &mut Context<Self>) {
        let Some(pending) = self.link.pending.take_if(|pending| pending.point == p) else {
            return;
        };
        // A range not about `p` would be asked about again and again.
        let linked = range.is_some();
        let range = range.filter(|r| char_in(r, p)).unwrap_or(pending.word);
        self.link.cached = Some(Answer { range, linked });
        self.update_link(cx);
    }

    fn set_link_shown(&mut self, shown: Option<Range<Point>>, cx: &mut Context<Self>) {
        if self.link.shown != shown {
            self.link.shown = shown;
            cx.notify();
        }
    }
}

/// The columns of `row` inside `range`; `u32::MAX` ends a row the range
/// goes on past.
pub(super) fn row_columns(range: &Range<Point>, row: u32) -> Option<Range<u32>> {
    if row < range.start.row || row > range.end.row {
        return None;
    }
    let start = if row == range.start.row {
        range.start.column
    } else {
        0
    };
    let end = if row == range.end.row {
        range.end.column
    } else {
        u32::MAX
    };
    (start < end).then_some(start..end)
}

/// Whether the char at `p` is in `range` (an empty range holds its start).
fn char_in(range: &Range<Point>, p: Point) -> bool {
    let key = |p: Point| (p.row, p.column);
    let (start, end, p) = (key(range.start), key(range.end), key(p));
    start <= p && (p < end || p == start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    #[test]
    fn row_columns_split_a_range_by_row() {
        let r = pt(1, 4)..pt(3, 2);
        assert_eq!(row_columns(&r, 0), None);
        assert_eq!(row_columns(&r, 1), Some(4..u32::MAX));
        assert_eq!(row_columns(&r, 2), Some(0..u32::MAX));
        assert_eq!(row_columns(&r, 3), Some(0..2));
        assert_eq!(row_columns(&(pt(0, 3)..pt(0, 3)), 0), None);
    }

    #[test]
    fn char_in_excludes_the_end() {
        let r = pt(1, 4)..pt(1, 13);
        assert!(char_in(&r, pt(1, 4)) && char_in(&r, pt(1, 12)));
        assert!(!char_in(&r, pt(1, 13)) && !char_in(&r, pt(1, 3)));
    }
}
