//! A minimal notification at the bottom right of the window: a message,
//! one action button and a close button. It closes after
//! [`TOAST_DURATION`]. Used to offer an installed extension's theme
//! (spec §5.2).

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
    action_label: SharedString,
    action: ToastAction,
    /// Tells this toast's timer from a later toast's.
    id: u64,
}

impl Toast {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn action_label(&self) -> &str {
        &self.action_label
    }
}

impl Workbench {
    /// The toast shown, if any.
    pub fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    /// Shows a toast, replacing any shown one; it closes after
    /// [`TOAST_DURATION`].
    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        action_label: impl Into<SharedString>,
        action: ToastAction,
        cx: &mut Context<Self>,
    ) {
        self.toast_count += 1;
        let id = self.toast_count;
        self.toast = Some(Toast {
            message: message.into(),
            action_label: action_label.into(),
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
        if let Some(toast) = self.toast.take() {
            (toast.action)(self, window, cx);
            cx.notify();
        }
    }

    /// After the extension service changed: offers the theme of each
    /// extension installed since last time, once the index lists it.
    pub(crate) fn offer_installed_themes(&mut self, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let installs = service.fresh_installs();
        let mut offer = None;
        while let Some(id) = installs.get(self.fresh_installs_seen) {
            let Some(ext) = service.index().extensions.get(id) else {
                // Its index has not arrived yet.
                break;
            };
            self.fresh_installs_seen += 1;
            if let Some(theme) = ext.themes.first() {
                offer = Some((theme.name.clone(), false));
            } else if let Some(theme) = ext.icon_themes.first() {
                offer = Some((theme.name.clone(), true));
            }
        }
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
    let button = div()
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
        .child(toast.action_label.clone());
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
                .child(button)
                .child(close),
        )
}
