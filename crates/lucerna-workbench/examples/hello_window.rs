//! Opens an 800x600 window showing "Lucerna". Used to verify the GPUI pin.
use gpui::{
    App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct Hello;

impl Render for Hello {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .justify_center()
            .items_center()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .text_xl()
            .child("Lucerna")
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Hello),
        )
        .unwrap();
        cx.activate(true);
    });
}
