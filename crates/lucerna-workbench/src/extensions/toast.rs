//! A minimal notification at the bottom right of the window: a message,
//! at most one action button and a close button. It closes after
//! [`TOAST_DURATION`]. Used to offer an installed extension's theme
//! (spec §5.2) and to say why Clear Build Cache did nothing.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Context, IntoElement, MouseButton, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px, svg,
};

use crate::extensions::ExtensionService;
use crate::layout::color;
use crate::theme_picker::set_theme_setting;
use crate::ui::{CLOSE_ICON, GRID, POPOVER_RADIUS, STATUS_BAR_HEIGHT, UI_FONT_SIZE_SMALL};
use crate::{AppState, Workbench};

/// How long a toast stays.
pub const TOAST_DURATION: Duration = Duration::from_secs(10);

/// What the action button runs.
type ToastAction = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

pub struct Toast {
    message: SharedString,
    action: Option<(SharedString, ToastAction)>,
    /// Tells this toast's timer from a later toast's.
    id: u64,
}

impl Toast {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn action_label(&self) -> Option<&str> {
        self.action.as_ref().map(|(label, _)| label.as_ref())
    }
}

impl Workbench {
    /// The toast shown, if any.
    pub fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    /// Shows a toast with an action button labelled `action_label`,
    /// replacing any shown one; it closes after [`TOAST_DURATION`].
    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        action_label: impl Into<SharedString>,
        action: ToastAction,
        cx: &mut Context<Self>,
    ) {
        self.set_toast(message.into(), Some((action_label.into(), action)), cx);
    }

    /// Shows a toast with only a message (see [`Workbench::show_toast`]).
    pub(crate) fn show_message_toast(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.set_toast(message.into(), None, cx);
    }

    fn set_toast(
        &mut self,
        message: SharedString,
        action: Option<(SharedString, ToastAction)>,
        cx: &mut Context<Self>,
    ) {
        self.toast_count += 1;
        let id = self.toast_count;
        self.toast = Some(Toast {
            message,
            action,
            id,
        });
        cx.spawn(async move |wb, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            wb.update(cx, |wb, cx| {
                if wb.toast.as_ref().is_some_and(|t| t.id == id) {
                    wb.dismiss_toast(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        if self.toast.take().is_some() {
            cx.notify();
        }
    }

    fn run_toast_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, action)) = self.toast.take().and_then(|t| t.action) {
            action(self, window, cx);
        }
        cx.notify();
    }

    /// After the extension service changed: offers the theme of each
    /// extension installed since last time, once the index lists it. One
    /// whose index entry has not arrived yet waits without holding back
    /// the others.
    pub(crate) fn offer_installed_themes(&mut self, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let installs = service.fresh_installs();
        let new = installs.get(self.fresh_installs_seen..).unwrap_or_default();
        self.fresh_installs_seen = installs.len();
        self.fresh_installs_waiting.extend(new.iter().cloned());
        let mut offer = None;
        self.fresh_installs_waiting.retain(|id| {
            let Some(ext) = service.index().extensions.get(id) else {
                return true;
            };
            if let Some(theme) = ext.themes.first() {
                offer = Some((theme.name.clone(), false));
            } else if let Some(theme) = ext.icon_themes.first() {
                offer = Some((theme.name.clone(), true));
            }
            false
        });
        if let Some((name, icons)) = offer {
            let action_name = name.clone();
            self.show_toast(
                format!("Use theme “{name}”?"),
                "Set Theme",
                Rc::new(move |_, _, cx| {
                    // A failure shows in the status bar.
                    let _ = set_theme_setting(&action_name, icons, cx);
                }),
                cx,
            );
        }
    }
}

/// The shown toast, placed at the window's bottom right above the status
/// bar. Give it a `relative` parent the size of the window.
pub(crate) fn render(toast: &Toast, cx: &mut Context<Workbench>) -> impl IntoElement + use<> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    let button = toast.action.as_ref().map(|(label, _)| {
        div()
            .id("toast-action")
            .debug_selector(|| "toast-action".into())
            .flex_none()
            .px(px(2. * GRID))
            .py(px(GRID / 2.))
            .rounded(px(GRID))
            .border_1()
            .border_color(color(ui.border_variant))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(|wb, _, window, cx| wb.run_toast_action(window, cx)))
            .child(label.clone())
    });
    let close = div()
        .id("toast-close")
        .debug_selector(|| "toast-close".into())
        .flex_none()
        .p(px(GRID / 2.))
        .rounded(px(GRID))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(|wb, _, _, cx| wb.dismiss_toast(cx)))
        .child(
            svg()
                .data(CLOSE_ICON)
                .size(px(12.))
                .text_color(color(ui.icon_muted)),
        );
    div()
        .absolute()
        .right(px(3. * GRID))
        .bottom(px(STATUS_BAR_HEIGHT + 3. * GRID))
        .child(
            div()
                .id("toast")
                .debug_selector(|| "toast".into())
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .flex()
                .items_center()
                .gap(px(2. * GRID))
                .max_w(px(360.))
                .pl(px(3. * GRID))
                .pr(px(2. * GRID))
                .py(px(2. * GRID))
                .rounded(px(POPOVER_RADIUS))
                .border_1()
                .border_color(color(ui.border))
                .bg(color(ui.elevated_surface_background))
                .text_color(color(ui.text))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .shadow_lg()
                .child(div().flex_1().min_w_0().child(toast.message.clone()))
                .children(button)
                .child(close),
        )
}
