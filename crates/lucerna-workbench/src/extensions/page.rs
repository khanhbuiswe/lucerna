//! The Extensions page in an editor tab (spec §5.2): a search input and
//! filter buttons over a virtualised list of cards, installed extensions
//! first. Each card shows the name, version and id; the description
//! (fetched with `describe` the first time an uninstalled card is laid
//! out); what the extension holds, or its repository and commit; its
//! buttons; and while installing, the stage, after a failure, the error
//! with "Show Log", and the last install's warnings.

use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, IntoElement, ListAlignment,
    ListOffset, ListState, MouseButton, ParentElement, Render, SharedString, Styled, Subscription,
    WeakEntity, Window, div, list, prelude::*, px,
};

use super::ExtensionService;
use super::panel_model::{
    Filter, PanelRows, Row, RowState, rows, selected_filter, stage_label, with_filter,
};
use crate::layout::color;
use crate::text_input::TextInput;
use crate::ui::{GRID, UI_FONT_SIZE, UI_FONT_SIZE_CAPTION, UI_FONT_SIZE_SMALL, text_button};
use crate::{AppState, OpenMode, Workbench};

pub(crate) const LOADING_LIST: &str = "Loading extension list…";
pub(crate) const GIT_MISSING: &str =
    "git was not found. Install the Xcode Command Line Tools: xcode-select --install";
pub(crate) const FETCH_FAILED: &str = "Could not fetch the extension list.";
pub(crate) const LSP_NOTE: &str = "Language server: Phase 3";
pub(crate) const LOADING_DESCRIPTION: &str = "Loading description…";
pub(crate) const NO_DESCRIPTION: &str = "No description";
pub(crate) const CLEAR_CACHE_BUSY: &str =
    "Can't clear the build cache while an extension is installing.";

/// The filter buttons, left to right: label, debug-selector name and the
/// filter they set (`None` clears every filter).
const FILTERS: [(&str, &str, Option<Filter>); 6] = [
    ("All", "all", None),
    ("Installed", "installed", Some(Filter::Installed)),
    ("Updates", "updates", Some(Filter::Updates)),
    ("Themes", "themes", Some(Filter::Themes)),
    ("Icon Themes", "icon-themes", Some(Filter::Icons)),
    ("Languages", "languages", Some(Filter::Languages)),
];

/// The filter buttons' labels, left to right.
pub const FILTER_LABELS: [&str; 6] = [
    FILTERS[0].0,
    FILTERS[1].0,
    FILTERS[2].0,
    FILTERS[3].0,
    FILTERS[4].0,
    FILTERS[5].0,
];

/// Rendered beyond the visible cards, so scrolling does not pop them in.
const LIST_OVERDRAW: f32 = 50. * GRID;

const CARD_RADIUS: f32 = 6.;

/// Which of [`PanelRows`]' groups a card's row is in, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Card {
    Installed(usize),
    Available(usize),
}

pub struct ExtensionsPage {
    workbench: WeakEntity<Workbench>,
    focus: FocusHandle,
    query: Entity<TextInput>,
    /// The query the rows were last computed for: the input's text as of
    /// its last change (the input cannot be read while it reports one).
    shown_query: String,
    rows: PanelRows,
    /// The cards in list order: installed, then available.
    cards: Vec<Card>,
    list: ListState,
    /// The uninstalled cards without a description laid out this frame:
    /// `(list index, id)`.
    laid_out: Vec<(usize, String)>,
    /// The page was shown before `registry.json` was read: fetch the
    /// registry if it turns out to be missing.
    fetch_when_loaded: bool,
    _service: Option<Subscription>,
}

impl ExtensionsPage {
    pub(crate) fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let query = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Search extensions (@installed, @updates, @themes…)");
            input.on_change(move |text, _, cx| {
                let text = text.to_string();
                this.update(cx, |page, cx| page.refresh_for(text, cx)).ok();
            });
            input
        });
        let service = cx
            .has_global::<ExtensionService>()
            .then(|| cx.observe_global::<ExtensionService>(Self::service_changed));
        let mut page = ExtensionsPage {
            workbench,
            focus: cx.focus_handle(),
            query,
            shown_query: String::new(),
            rows: PanelRows::default(),
            cards: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            laid_out: Vec::new(),
            fetch_when_loaded: false,
            _service: service,
        };
        page.refresh(cx);
        page
    }

    /// The rows shown for the current query, with what runs on them.
    pub fn rows(&self) -> &PanelRows {
        &self.rows
    }

    pub fn query_input(&self) -> &Entity<TextInput> {
        &self.query
    }

    /// Puts the cursor in the search input.
    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.query.focus_handle(cx), cx);
    }

    /// The page was opened or focused: fetches the registry when there is
    /// none and no fetch ran or failed yet.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        if !service.registry_loaded() {
            self.fetch_when_loaded = true;
        } else if service.registry().is_none()
            && !service.is_refreshing()
            && service.registry_error().is_none()
        {
            ExtensionService::refresh_registry(cx);
        }
    }

    fn service_changed(&mut self, cx: &mut Context<Self>) {
        if self.fetch_when_loaded
            && cx
                .try_global::<ExtensionService>()
                .is_some_and(ExtensionService::registry_loaded)
        {
            self.fetch_when_loaded = false;
            self.shown(cx);
        }
        self.refresh(cx);
    }

    /// Recomputes the rows from the service for the current query.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_for(self.shown_query.clone(), cx);
    }

    /// Recomputes the rows from the service and `query`. A new query
    /// scrolls to the top; otherwise the card at the top stays there.
    fn refresh_for(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let mut rows = rows(
            service.registry().map(|r| r.as_ref()),
            service.index(),
            service.updates(),
            &query,
        );
        rows.overlay(|row| {
            if service.is_installing(&row.id) {
                let stage = service
                    .progress(&row.id)
                    .map(|p| p.stage.clone())
                    .unwrap_or(lucerna_extension::builder::Stage::Fetching);
                Some(RowState::Installing(stage))
            } else {
                service
                    .last_error(&row.id)
                    .map(|e| RowState::Failed(e.to_string()))
            }
        });
        let cards: Vec<Card> = (0..rows.installed.len())
            .map(Card::Installed)
            .chain((0..rows.available.len()).map(Card::Available))
            .collect();
        let ids = |rows: &PanelRows, cards: &[Card]| -> Vec<String> {
            cards
                .iter()
                .filter_map(|&c| card_row(rows, c))
                .map(|r| r.id.clone())
                .collect()
        };
        let (old_ids, new_ids) = (ids(&self.rows, &self.cards), ids(&rows, &cards));
        if query != self.shown_query {
            self.list.reset(cards.len());
        } else if old_ids == new_ids {
            self.list.remeasure_items(0..cards.len());
        } else {
            let top = self.list.logical_scroll_top();
            let anchor = old_ids.get(top.item_ix);
            self.list.reset(cards.len());
            if let Some(item_ix) = anchor.and_then(|id| new_ids.iter().position(|n| n == id)) {
                self.list.scroll_to(ListOffset {
                    item_ix,
                    offset_in_item: top.offset_in_item,
                });
            }
        }
        self.shown_query = query;
        self.rows = rows;
        self.cards = cards;
        cx.notify();
    }

    /// Sets the query's filter token (see [`with_filter`]).
    fn set_filter(&mut self, filter: Option<Filter>, window: &mut Window, cx: &mut Context<Self>) {
        let text = with_filter(&self.shown_query, filter);
        self.query
            .update(cx, |input, cx| input.set_text(&text, window, cx));
        self.refresh_for(text, cx);
    }

    /// After a frame: asks the service for the descriptions of the cards
    /// laid out without one that are on screen (not in the overdraw), top
    /// first. That replaces the service's describe queue, so cards that
    /// scrolled away are dropped from it (see
    /// [`ExtensionService::want_descriptions`]).
    fn request_descriptions(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<ExtensionService>() {
            return;
        }
        let mut visible: Vec<(usize, String)> = std::mem::take(&mut self.laid_out)
            .into_iter()
            .filter(|(ix, _)| {
                self.list.item_is_above_viewport(*ix) != Some(true)
                    && self.list.item_is_below_viewport(*ix) != Some(true)
            })
            .collect();
        visible.sort();
        visible.dedup();
        let ids: Vec<String> = visible.into_iter().map(|(_, id)| id).collect();
        ExtensionService::want_descriptions(&ids, cx);
    }

    fn show_log(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let path: Arc<Path> = service
            .store()
            .root()
            .join("logs")
            .join(format!("{id}.log"))
            .into();
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |wb, cx| wb.open_path(path, OpenMode::Preview, cx));
        }
    }

    fn render_card(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(&card) = self.cards.get(ix) else {
            return div().into_any_element();
        };
        let Some(row) = card_row(&self.rows, card).cloned() else {
            return div().into_any_element();
        };
        let installed = matches!(card, Card::Installed(_));
        if !installed && row.description.is_none() && row.state == RowState::NotInstalled {
            self.laid_out.push((ix, row.id.clone()));
        }
        let ui = cx.global::<AppState>().theme.ui.clone();
        let service = cx.try_global::<ExtensionService>();
        // A describe ran and brought nothing (or failed), and none runs or
        // waits now.
        let described_nothing = service.is_some_and(|s| {
            s.describe_started(&row.id).is_some()
                && !s.is_describing(&row.id)
                && !s.queued_describes().any(|q| q == row.id)
        });
        let has_update = service.is_some_and(|s| s.updates().contains(&row.id));
        let warnings: Vec<String> = service.map_or_else(Vec::new, |s| s.warnings(&row.id).to_vec());
        let (muted, error) = (color(ui.text_muted), color(ui.error));
        let id = row.id.clone();

        let title = div()
            .flex()
            .items_baseline()
            .gap(px(2. * GRID))
            .min_w_0()
            .child(
                div()
                    .flex_none()
                    .text_size(px(UI_FONT_SIZE))
                    .child(row.title().to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(UI_FONT_SIZE_CAPTION))
                    .text_color(muted)
                    .child(format!("v{}", row.version)),
            )
            .child(
                text_cell(row.id.clone())
                    .text_size(px(UI_FONT_SIZE_CAPTION))
                    .text_color(muted),
            );
        let description = match &row.description {
            Some(text) => text.clone(),
            None if installed => NO_DESCRIPTION.into(),
            None if described_nothing => NO_DESCRIPTION.into(),
            None => LOADING_DESCRIPTION.into(),
        };
        let details = if installed {
            let mut parts: Vec<String> = Vec::new();
            if !row.summary.is_empty() {
                parts.push(row.summary.clone());
            }
            if row.lsp_note {
                parts.push(LSP_NOTE.into());
            }
            parts.join(" · ")
        } else {
            let commit: Option<String> = row.commit.as_ref().map(|c| c.chars().take(7).collect());
            [row.repository.clone(), commit]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ")
        };

        let mut buttons: Vec<AnyElement> = Vec::new();
        let mut button = |kind: &'static str, label: &'static str, cx: &mut Context<Self>| {
            let id = id.clone();
            let element = text_button(format!("extension-{kind}-{id}"), label, cx)
                .on_click(cx.listener(move |page, _, _, cx| {
                    cx.stop_propagation();
                    match kind {
                        "cancel" => ExtensionService::cancel(id.clone(), cx),
                        "uninstall" => ExtensionService::uninstall(id.clone(), cx),
                        _ => ExtensionService::install(id.clone(), cx),
                    }
                    page.refresh(cx);
                }))
                .into_any_element();
            buttons.push(element);
        };
        match (&row.state, installed) {
            (RowState::Installing(_), _) => button("cancel", "Cancel", cx),
            (_, false) => button("install", "Install", cx),
            (RowState::Broken(_), true) => {
                button("reinstall", "Reinstall", cx);
                button("uninstall", "Uninstall", cx);
            }
            (_, true) => {
                if has_update {
                    button("update", "Update", cx);
                }
                button("uninstall", "Uninstall", cx);
            }
        }

        // "Show Log" goes under the action buttons, flush right with them.
        let show_log = matches!(row.state, RowState::Failed(_)).then(|| {
            let log_id = id.clone();
            text_button(format!("extension-show-log-{id}"), "Show Log", cx).on_click(cx.listener(
                move |page, _, _, cx| {
                    cx.stop_propagation();
                    page.show_log(&log_id, cx);
                },
            ))
        });
        let status: Option<AnyElement> = match &row.state {
            RowState::Installing(stage) => Some(
                line(format!("extension-progress-{id}"))
                    .text_color(color(ui.text))
                    .child(text_cell(stage_label(stage)))
                    .into_any_element(),
            ),
            RowState::Failed(message) => Some(
                line(format!("extension-error-{id}"))
                    .child(text_cell(message.clone()).text_color(error))
                    .into_any_element(),
            ),
            RowState::Broken(reason) => Some(
                line(format!("extension-broken-{id}"))
                    .child(text_cell(reason.clone()).text_color(error))
                    .into_any_element(),
            ),
            _ => None,
        };
        // The last install's warnings, once it is done.
        let done = matches!(row.state, RowState::Installed | RowState::UpdateAvailable);
        let warnings = (done && !warnings.is_empty()).then(|| {
            div()
                .debug_selector({
                    let id = id.clone();
                    move || format!("extension-warnings-{id}")
                })
                .flex()
                .flex_col()
                .children(
                    warnings
                        .into_iter()
                        .map(|w| line(String::new()).child(text_cell(format!("⚠ {w}")))),
                )
        });

        let hover = color(ui.ghost_element_hover);
        let selector = format!("extension-card-{id}");
        let item_selector = format!("extension-item-{id}");
        div()
            .debug_selector(move || item_selector)
            .px(px(4. * GRID))
            .pb(px(2. * GRID))
            .child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .flex()
                    .gap(px(3. * GRID))
                    .p(px(3. * GRID))
                    .rounded(px(CARD_RADIUS))
                    .border_1()
                    .border_color(color(ui.border_variant))
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(GRID))
                            .child(title)
                            .child(
                                truncated(description)
                                    .text_size(px(UI_FONT_SIZE_SMALL))
                                    .text_color(muted),
                            )
                            .when(!details.is_empty(), |d| {
                                d.child(
                                    truncated(details)
                                        .text_size(px(UI_FONT_SIZE_CAPTION))
                                        .text_color(muted),
                                )
                            })
                            .children(status)
                            .children(warnings),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .items_end()
                            .gap(px(GRID))
                            .child(div().flex().gap(px(GRID)).children(buttons))
                            .children(show_log),
                    ),
            )
            .into_any_element()
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> Div {
        let ui = &cx.global::<AppState>().theme.ui;
        let selected = selected_filter(&self.shown_query);
        let (hover, active_bg) = (color(ui.ghost_element_hover), color(ui.element_selected));
        let (text, muted) = (color(ui.text), color(ui.text_muted));
        let buttons: Vec<AnyElement> = FILTERS
            .iter()
            .map(|&(label, name, filter)| {
                let active = selected == Some(filter);
                let selector = format!("extensions-filter-{name}");
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(6. * GRID))
                    .px(px(2. * GRID))
                    .rounded(px(GRID))
                    .cursor_pointer()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    // Keeps the cursor in the search input.
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(
                        cx.listener(move |page, _, window, cx| page.set_filter(filter, window, cx)),
                    )
                    .when(active, |d| {
                        d.bg(active_bg).text_color(text).child(
                            div()
                                .debug_selector(move || format!("extensions-filter-{name}-active"))
                                .child(label),
                        )
                    })
                    .when(!active, |d| {
                        d.text_color(muted).hover(move |s| s.bg(hover)).child(label)
                    })
                    .into_any_element()
            })
            .collect();
        div().flex().flex_wrap().gap(px(GRID)).children(buttons)
    }

    /// The message under the header: the registry is loading, git is
    /// missing, or the fetch failed (with Retry).
    fn render_message(&self, cx: &mut Context<Self>) -> Option<Div> {
        let service = cx.try_global::<ExtensionService>()?;
        let ui = &cx.global::<AppState>().theme.ui;
        let message = |selector: &'static str, text: &'static str| {
            div()
                .debug_selector(move || selector.into())
                .flex()
                .items_center()
                .gap(px(2. * GRID))
                .px(px(4. * GRID))
                .pb(px(2. * GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.text_muted))
                .child(div().flex_1().min_w_0().child(text))
        };
        if service.is_refreshing() && service.registry().is_none() {
            Some(message("extensions-loading", LOADING_LIST))
        } else if service.git_missing() {
            Some(message("extensions-git-missing", GIT_MISSING))
        } else if service.registry_error().is_some() {
            Some(
                message("extensions-registry-error", FETCH_FAILED).child(
                    text_button("extensions-retry".to_string(), "Retry", cx)
                        .on_click(|_, _, cx| ExtensionService::refresh_registry(cx)),
                ),
            )
        } else if service.registry().is_none() {
            Some(message("extensions-loading", LOADING_LIST))
        } else {
            None
        }
    }
}

fn card_row(rows: &PanelRows, card: Card) -> Option<&Row> {
    match card {
        Card::Installed(i) => rows.installed.get(i),
        Card::Available(i) => rows.available.get(i),
    }
}

/// A line of a card that holds text and maybe a button.
fn line(selector: String) -> Div {
    div()
        .when(!selector.is_empty(), |d| d.debug_selector(move || selector))
        .flex()
        .items_center()
        .gap(px(2. * GRID))
        .text_size(px(UI_FONT_SIZE_CAPTION))
}

/// One line of text that ends in an ellipsis when too long.
fn truncated(text: String) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

/// [`truncated`] text that takes the rest of a row.
fn text_cell(text: String) -> Div {
    truncated(text).flex_1()
}

impl Focusable for ExtensionsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ExtensionsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The list lays its cards out after this; once the frame is done,
        // the visible ones without a description are asked for.
        self.laid_out.clear();
        let page = cx.weak_entity();
        cx.defer(move |cx| {
            page.update(cx, |page, cx| page.request_descriptions(cx))
                .ok();
        });
        let ui = cx.global::<AppState>().theme.ui.clone();
        let search = div()
            .debug_selector(|| "extensions-search".into())
            .flex()
            .items_center()
            .h(px(8. * GRID))
            .px(px(3. * GRID))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(color(ui.border_variant))
            .bg(color(ui.editor_background))
            .text_size(px(UI_FONT_SIZE))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        let header = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2. * GRID))
            .px(px(4. * GRID))
            .pt(px(4. * GRID))
            .pb(px(3. * GRID))
            .child(search)
            .child(self.render_filters(cx));
        let message = self.render_message(cx);
        let this = cx.weak_entity();
        let cards = list(self.list.clone(), move |ix, _, cx| {
            this.update(cx, |page, cx| page.render_card(ix, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        div()
            .id("extensions-page")
            .debug_selector(|| "extensions-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .child(header)
            .children(message)
            .child(
                div()
                    .debug_selector(|| "extensions-list".into())
                    .flex_1()
                    .min_h_0()
                    .child(cards),
            )
    }
}
