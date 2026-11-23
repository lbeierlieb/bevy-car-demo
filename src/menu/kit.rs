//! The menus' look: panels with a title and a prompt line, and big rows (buttons,
//! toggles, choices, sliders) that work with six buttons as well as with the mouse. Left
//! and right change a focused toggle, choice or slider; choose clicks a button or
//! toggles.

use bevy_egui::egui::{
    self, Align2, Color32, CornerRadius, EventFilter, FontFamily, FontId, Key, Rect, Response,
    Sense, Stroke, Ui, pos2, vec2,
};

use super::nav::{self, Device};
use super::{ACCENT, MEDIUM, PANEL};

/// A row's height and its text size (points).
pub const ROW: f32 = 40.0;
const ROW_TEXT: f32 = 17.0;
const TITLE: f32 = 25.0;
const MARGIN: f32 = 18.0;

const ROW_FILL: Color32 = Color32::from_rgb(34, 37, 43);
const ROW_HOVER: Color32 = Color32::from_rgb(48, 52, 60);
const ROW_FOCUS: Color32 = Color32::from_rgb(74, 58, 30);
const TEXT: Color32 = Color32::from_rgb(236, 236, 232);
const WEAK: Color32 = Color32::from_rgb(150, 152, 156);
const ON: Color32 = Color32::from_rgb(90, 190, 110);

/// Where a panel sits.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Place {
    /// In the middle of the window (the central menu, the start menu).
    Center,
    /// At the right edge, leaving the scene in view (shops, lobbies).
    Right,
}

/// How menus outrank each other for the focus (see `nav::claim`).
pub mod rank {
    pub const WINDOW: u8 = 10;
    pub const START: u8 = 20;
    pub const CENTRAL: u8 = 30;
    pub const INTRO: u8 = 40;
}

/// A panel: title, then the contents (scrolling when they don't fit), then the prompt
/// line. It claims the focus at its rank.
pub struct Panel<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub place: Place,
    pub rank: u8,
    pub width: f32,
    /// What "back" does here, for the prompt line (none: no back).
    pub back: Option<&'a str>,
}

impl<'a> Panel<'a> {
    pub fn new(id: &'a str, title: &'a str) -> Self {
        Self {
            id,
            title,
            place: Place::Center,
            rank: rank::WINDOW,
            width: 440.0,
            back: Some("Back"),
        }
    }

    pub fn place(mut self, place: Place) -> Self {
        self.place = place;
        self
    }

    pub fn rank(mut self, rank: u8) -> Self {
        self.rank = rank;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn back(mut self, back: Option<&'a str>) -> Self {
        self.back = back;
        self
    }

    pub fn show<R>(self, ctx: &egui::Context, contents: impl FnOnce(&mut Ui) -> R) -> R {
        let screen = ctx.content_rect();
        let width = self.width.min(screen.width() - 2.0 * MARGIN);
        let (anchor, offset) = match self.place {
            Place::Center => (Align2::CENTER_CENTER, vec2(0.0, 0.0)),
            Place::Right => (Align2::RIGHT_TOP, vec2(-MARGIN, MARGIN)),
        };
        let most_height = screen.height() - 2.0 * MARGIN;
        egui::Area::new(egui::Id::new(self.id))
            .order(egui::Order::Foreground)
            .anchor(anchor, offset)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, Color32::from_gray(58)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(width - 32.0);
                        ui.set_max_height(most_height - 32.0);
                        nav::claim(ui, self.rank);
                        ui.label(
                            egui::RichText::new(self.title)
                                .font(FontId::new(TITLE, FontFamily::Name(MEDIUM.into())))
                                .color(TEXT),
                        );
                        ui.add_space(6.0);
                        let prompt_height = 26.0;
                        let inner = egui::ScrollArea::vertical()
                            .max_height(most_height - 32.0 - TITLE - 16.0 - prompt_height)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                contents(ui)
                            })
                            .inner;
                        ui.add_space(6.0);
                        prompt(ui, self.back);
                        inner
                    })
                    .inner
            })
            .inner
    }
}

/// The device the menus were last worked with (as `nav` saw it).
pub fn device(ctx: &egui::Context) -> Device {
    nav::device(ctx)
}

/// "Enter Choose   Esc Back", named for the device in use.
fn prompt(ui: &mut Ui, back: Option<&str>) {
    let (choose, go_back) = match device(ui.ctx()) {
        Device::Keyboard => ("Enter", "Esc"),
        Device::Mouse => ("Click", "Esc"),
        Device::Pad => ("A", "B"),
        Device::Wheel => ("Choose button", "Back button"),
    };
    let mut job = egui::text::LayoutJob::default();
    let key = egui::TextFormat::simple(FontId::new(13.0, FontFamily::Name(MEDIUM.into())), ACCENT);
    let text = egui::TextFormat::simple(FontId::proportional(13.0), WEAK);
    job.append(choose, 0.0, key.clone());
    job.append("Choose", 6.0, text.clone());
    if let Some(back) = back {
        job.append(go_back, 22.0, key);
        job.append(back, 6.0, text);
    }
    ui.label(job);
}

/// The common part of a row: its rect and response (focusable), its background painted.
fn row(ui: &mut Ui, height: f32, sense: Sense) -> (Rect, Response) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let fill = if response.has_focus() {
        ROW_FOCUS
    } else if response.hovered() && ui.is_enabled() {
        ROW_HOVER
    } else {
        ROW_FILL
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    (rect, response)
}

fn text_color(ui: &Ui) -> Color32 {
    if ui.is_enabled() { TEXT } else { WEAK }
}

fn row_font() -> FontId {
    FontId::new(ROW_TEXT, FontFamily::Name(MEDIUM.into()))
}

/// Keeps left and right for the focused row (they change it rather than moving the
/// focus), and says which was pressed: -1, 0 or 1.
fn sideways(ui: &Ui, response: &Response) -> i32 {
    if !response.has_focus() {
        return 0;
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            response.id,
            EventFilter {
                horizontal_arrows: true,
                ..Default::default()
            },
        );
    });
    ui.input(|i| i.num_presses(Key::ArrowRight) as i32 - i.num_presses(Key::ArrowLeft) as i32)
}

/// A button as wide as the panel. `detail` is shown weaker on the right.
pub fn button(ui: &mut Ui, label: &str) -> Response {
    button_with(ui, label, "")
}

pub fn button_with(ui: &mut Ui, label: &str, detail: &str) -> Response {
    let (rect, response) = row(ui, ROW, Sense::click());
    let painter = ui.painter();
    let y = rect.center().y;
    painter.text(
        pos2(rect.left() + 14.0, y),
        Align2::LEFT_CENTER,
        label,
        row_font(),
        text_color(ui),
    );
    if !detail.is_empty() {
        painter.text(
            pos2(rect.right() - 14.0, y),
            Align2::RIGHT_CENTER,
            detail,
            FontId::proportional(15.0),
            WEAK,
        );
    }
    response
}

/// A row that goes on to another screen (a chevron on the right).
pub fn submenu(ui: &mut Ui, label: &str) -> Response {
    let response = button(ui, label);
    let r = response.rect;
    let c = pos2(r.right() - 18.0, r.center().y);
    ui.painter().add(egui::Shape::line(
        vec![c + vec2(-4.0, -6.0), c, c + vec2(-4.0, 6.0)],
        Stroke::new(2.0, WEAK),
    ));
    response
}

/// A row with a switch: choose (or a click) flips it, left turns it off, right on.
pub fn toggle(ui: &mut Ui, label: &str, on: &mut bool) -> Response {
    let (rect, mut response) = row(ui, ROW, Sense::click());
    let old = *on;
    match sideways(ui, &response) {
        -1 => *on = false,
        1 => *on = true,
        _ => {}
    }
    if response.clicked() {
        *on ^= true;
    }
    if *on != old {
        response.mark_changed();
    }
    let painter = ui.painter();
    let y = rect.center().y;
    painter.text(
        pos2(rect.left() + 14.0, y),
        Align2::LEFT_CENTER,
        label,
        row_font(),
        text_color(ui),
    );
    // The switch: a pill with a knob.
    let pill = Rect::from_center_size(pos2(rect.right() - 38.0, y), vec2(44.0, 22.0));
    painter.rect_filled(pill, 11.0, if *on { ON } else { Color32::from_gray(80) });
    let knob = if *on {
        pill.right() - 11.0
    } else {
        pill.left() + 11.0
    };
    painter.circle_filled(pos2(knob, y), 8.0, Color32::WHITE);
    response
}

/// A row choosing one of `options`: left and right step through them, choose (or a
/// click) goes to the next.
pub fn choice(ui: &mut Ui, label: &str, index: &mut usize, options: &[String]) -> Response {
    let (rect, mut response) = row(ui, ROW, Sense::click());
    let count = options.len().max(1);
    let old = *index;
    let step = sideways(ui, &response);
    if step < 0 {
        *index = index.saturating_sub(1);
    } else if step > 0 {
        *index = (*index + 1).min(count - 1);
    }
    if response.clicked() {
        *index = (*index + 1) % count;
    }
    if *index != old {
        response.mark_changed();
    }
    let painter = ui.painter();
    let y = rect.center().y;
    painter.text(
        pos2(rect.left() + 14.0, y),
        Align2::LEFT_CENTER,
        label,
        row_font(),
        text_color(ui),
    );
    let value = options.get(*index).map_or("", String::as_str);
    let right = rect.right() - 14.0;
    let galley = painter.layout_no_wrap(value.to_string(), FontId::proportional(16.0), TEXT);
    let text_right = right - 16.0;
    let text_left = text_right - galley.size().x;
    painter.galley(pos2(text_left, y - galley.size().y / 2.0), galley, TEXT);
    arrow(painter, pos2(text_left - 12.0, y), -1.0, *index > 0);
    arrow(painter, pos2(right - 4.0, y), 1.0, *index + 1 < count);
    response
}

/// A small triangle pointing left (-1) or right (1), dimmed when it can't go further.
fn arrow(painter: &egui::Painter, at: egui::Pos2, side: f32, live: bool) {
    let color = if live { ACCENT } else { Color32::from_gray(70) };
    painter.add(egui::Shape::convex_polygon(
        vec![
            at + vec2(side * 5.0, 0.0),
            at + vec2(-side * 4.0, -6.0),
            at + vec2(-side * 4.0, 6.0),
        ],
        color,
        Stroke::NONE,
    ));
}

/// A row with a value and a bar: left and right step it by `step`, the mouse drags it.
pub fn slider(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    shown: impl Fn(f32) -> String,
) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut response) = row(ui, ROW + 10.0, Sense::click_and_drag());
    let old = *value;
    let steps = sideways(ui, &response);
    if steps != 0 {
        *value = ((*value / step).round() + steps as f32) * step;
    }
    let bar = Rect::from_min_max(
        pos2(rect.left() + 14.0, rect.bottom() - 13.0),
        pos2(rect.right() - 14.0, rect.bottom() - 9.0),
    );
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked())
    {
        let t = ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        *value = ((lo + t * (hi - lo)) / step).round() * step;
    }
    *value = value.clamp(lo, hi);
    if *value != old {
        response.mark_changed();
    }
    let painter = ui.painter();
    let y = rect.top() + 17.0;
    painter.text(
        pos2(rect.left() + 14.0, y),
        Align2::LEFT_CENTER,
        label,
        row_font(),
        text_color(ui),
    );
    painter.text(
        pos2(rect.right() - 14.0, y),
        Align2::RIGHT_CENTER,
        shown(*value),
        FontId::proportional(16.0),
        TEXT,
    );
    let t = if hi > lo {
        (*value - lo) / (hi - lo)
    } else {
        0.0
    };
    painter.rect_filled(bar, 2.0, Color32::from_gray(70));
    let filled = Rect::from_min_max(bar.min, pos2(bar.left() + t * bar.width(), bar.bottom()));
    painter.rect_filled(filled, 2.0, ACCENT);
    painter.circle_filled(pos2(filled.right(), bar.center().y), 6.0, Color32::WHITE);
    response
}

/// A heading between groups of rows.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text)
            .font(FontId::new(14.0, FontFamily::Name(MEDIUM.into())))
            .color(ACCENT),
    );
}

/// A line of explanation, weaker and wrapped.
pub fn note(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(14.0).color(WEAK));
}

/// A line of text (not focusable), with a value on the right.
pub fn info(ui: &mut Ui, label: &str, value: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let painter = ui.painter();
    let y = rect.center().y;
    painter.text(
        pos2(rect.left() + 14.0, y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(16.0),
        TEXT,
    );
    painter.text(
        pos2(rect.right() - 14.0, y),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(15.0),
        WEAK,
    );
}
