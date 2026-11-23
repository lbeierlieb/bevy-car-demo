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
                                nav::follow_focus(ui);
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

/// Whether the row has the focus (egui's `Response::has_focus` also wants the window to
/// have it, which a window under Xvfb never gets; keys only come with it anyway).
pub fn focused(ui: &Ui, response: &Response) -> bool {
    ui.memory(|m| m.has_focus(response.id))
}

/// The common part of a row: its rect and response (focusable), its background painted.
fn row(ui: &mut Ui, height: f32, sense: Sense) -> (Rect, Response) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let fill = if focused(ui, &response) {
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
    if !focused(ui, response) {
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

/// `choice` over any values: `options` in order, each named by `name`. Returns whether
/// the value changed.
pub fn pick<T: Copy + PartialEq>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    options: &[T],
    name: impl Fn(T) -> String,
) -> bool {
    let names: Vec<String> = options.iter().map(|&o| name(o)).collect();
    let mut index = options.iter().position(|o| o == value).unwrap_or(0);
    let changed = choice(ui, label, &mut index, &names).changed();
    if changed && let Some(&new) = options.get(index) {
        *value = new;
    }
    changed
}

/// A line of warning, in red.
pub fn warning(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(14.0)
            .color(Color32::from_rgb(255, 110, 90)),
    );
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

/// Colour swatches in rows of `per_row`, the picked one ringed in white. One widget for
/// the focus: the arrows move the pick (up from the top row and down from the bottom one
/// leave it), a click picks. Returns whether the pick changed.
pub fn swatches(
    ui: &mut Ui,
    colors: &[(&str, Color32)],
    per_row: usize,
    picked: &mut usize,
) -> bool {
    let gap = 8.0;
    let rows = colors.len().div_ceil(per_row);
    let width = (ui.available_width() - gap * (per_row as f32 - 1.0)) / per_row as f32;
    let height = 40.0;
    let size = vec2(ui.available_width(), rows as f32 * (height + gap) - gap);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let cell = |i: usize| {
        let (r, c) = (i / per_row, i % per_row);
        Rect::from_min_size(
            rect.min + vec2(c as f32 * (width + gap), r as f32 * (height + gap)),
            vec2(width, height),
        )
    };
    let old = *picked;
    if focused(ui, &response) && ui.is_enabled() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let (up, down, left, right) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
            )
        });
        let (r, c) = (*picked / per_row, *picked % per_row);
        if left && c > 0 {
            *picked -= 1;
        }
        if right && c + 1 < per_row && *picked + 1 < colors.len() {
            *picked += 1;
        }
        if up {
            if r == 0 {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Up));
            } else {
                *picked -= per_row;
            }
        }
        if down {
            if r + 1 >= rows || *picked + per_row >= colors.len() {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Down));
            } else {
                *picked += per_row;
            }
        }
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(i) = (0..colors.len()).find(|&i| cell(i).contains(pointer))
    {
        *picked = i;
    }
    let painter = ui.painter();
    for (i, (_, color)) in colors.iter().enumerate() {
        let r = cell(i);
        painter.rect_filled(r, 7.0, *color);
        if i == *picked {
            painter.rect_stroke(
                r.shrink(1.0),
                7.0,
                Stroke::new(3.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
    }
    if let Some(pointer) = response.hover_pos()
        && let Some(i) = (0..colors.len()).find(|&i| cell(i).contains(pointer))
    {
        response.clone().on_hover_text(colors[i].0);
    }
    *picked != old
}

/// A row with a bar filling up and what's going on.
pub fn progress(ui: &mut Ui, label: &str, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 7.0, ROW_FILL);
    let filled = Rect::from_min_max(
        rect.min,
        pos2(
            rect.left() + fraction.clamp(0.0, 1.0) * rect.width(),
            rect.bottom(),
        ),
    );
    painter.rect_filled(filled, 7.0, Color32::from_rgb(110, 80, 25));
    painter.text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        row_font(),
        TEXT,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Runs a pass with these keys pressed (and released), drawing `ui`.
    fn pass(ctx: &egui::Context, keys: &[Key], ui: &dyn Fn(&mut Ui)) {
        let mut input = egui::RawInput::default();
        for &key in keys {
            for pressed in [true, false] {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        let _ = ctx.run_ui(input, |root| {
            Panel::new("test", "Test").show(root.ctx(), ui);
        });
    }

    #[test]
    fn rows_work_with_the_arrows_and_enter() {
        let ctx = egui::Context::default();
        super::super::style(&ctx);
        nav::hook(&ctx);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let (on, volume, clicked) = (Cell::new(true), Cell::new(1.0), Cell::new(0));
        let draw = |ui: &mut Ui| {
            if button(ui, "First").clicked() {
                clicked.set(clicked.get() + 1);
            }
            let mut o = on.get();
            toggle(ui, "Sound", &mut o);
            on.set(o);
            let mut v = volume.get();
            slider(ui, "Volume", &mut v, 0.0..=1.0, 0.05, |v| format!("{v}"));
            volume.set(v);
        };
        // The focus goes to the first row; Enter clicks it.
        pass(&ctx, &[], &draw);
        pass(&ctx, &[], &draw);
        pass(&ctx, &[Key::Enter], &draw);
        assert_eq!(clicked.get(), 1);
        // Down to the toggle; left turns it off and doesn't move the focus.
        pass(&ctx, &[Key::ArrowDown], &draw);
        pass(&ctx, &[Key::ArrowLeft], &draw);
        pass(&ctx, &[], &draw);
        assert!(!on.get());
        // Down to the slider; left twice steps it down.
        pass(&ctx, &[Key::ArrowDown], &draw);
        pass(&ctx, &[Key::ArrowLeft], &draw);
        pass(&ctx, &[Key::ArrowLeft], &draw);
        assert!((volume.get() - 0.9).abs() < 1e-5, "{}", volume.get());
    }
}
