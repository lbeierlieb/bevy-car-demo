//! A response curve drawn as a graph (how far a control is pushed, across, against what
//! it asks for, up), its points edited with six buttons or the mouse: left and right
//! pick a point, choose grabs it, the arrows move it, choose keeps it and back puts it
//! back; the mouse drags points, a double click adds one and a right click removes one.
//! Under the graph: the presets, adding and removing points.

use bevy_egui::egui::{
    self, Align2, Color32, EventFilter, FontId, Key, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2,
    vec2,
};

use super::ACCENT;
use super::kit::{self, ON, ROW_FILL, ROW_FOCUS, TEXT, WEAK};
use super::nav::Device;
use crate::input::{Curve, Preset};

/// A grabbed point moves this far a press (of the whole range).
const STEP: f32 = 0.02;
/// The graph's side, at most.
const SIDE: f32 = 280.0;
/// How near (points) the pointer must be to a point to take it.
const REACH: f32 = 14.0;

/// What the editor keeps between passes.
#[derive(Clone, Copy, Default)]
struct State {
    selected: usize,
    /// Grabbed with the buttons, and where the point was.
    grab: Option<[f32; 2]>,
    /// Dragged with the mouse.
    dragging: bool,
}

/// What the editor did this pass.
pub struct Edited {
    pub changed: bool,
    /// A point is grabbed: "back" should come here (as `cancel`) rather than leave.
    pub grabbed: bool,
}

/// The graph of `curve`, with `live` (the control's push now, if bound) marked on it,
/// then the preset, adding and removing points. `cancel` puts a grabbed point back.
/// `salt` tells apart editors on different screens.
pub fn editor(
    ui: &mut Ui,
    salt: &str,
    curve: &mut Curve,
    live: Option<f32>,
    cancel: bool,
) -> Edited {
    let state_id = egui::Id::new(("curve editor", salt));
    let mut state: State = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let mut changed = false;
    let count = curve.points().len();
    state.selected = state.selected.min(count - 1);

    let side = (ui.available_width() - 190.0).clamp(120.0, SIDE);
    let size = vec2(ui.available_width(), side + 56.0);
    let (rect, response) = kit::allocate(ui, size, Sense::click_and_drag());
    let plot = Rect::from_min_size(rect.min + vec2(44.0, 22.0), vec2(side, side));
    let to_screen = |[x, y]: [f32; 2]| pos2(plot.left() + x * side, plot.bottom() - y * side);
    let from_screen = |p: Pos2| {
        [
            ((p.x - plot.left()) / side).clamp(0.0, 1.0),
            ((plot.bottom() - p.y) / side).clamp(0.0, 1.0),
        ]
    };
    let nearest = |curve: &Curve, at: Pos2| {
        (curve.points().iter().enumerate())
            .map(|(i, &p)| (i, to_screen(p).distance(at)))
            .filter(|&(_, d)| d < REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };

    // The buttons, while it has the focus.
    let focused = kit::focused(ui, &response);
    if !focused {
        state.grab = None;
    }
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    if focused {
        let presses = |key| ui.input(|i| i.num_presses(key) as f32);
        let across = presses(Key::ArrowRight) - presses(Key::ArrowLeft);
        let up = presses(Key::ArrowUp) - presses(Key::ArrowDown);
        if let Some(was) = state.grab {
            if cancel {
                curve.set_point(state.selected, was);
                state.grab = None;
                changed = true;
            } else {
                if across != 0.0 || up != 0.0 {
                    let [x, y] = curve.points()[state.selected];
                    curve.set_point(state.selected, [x + across * STEP, y + up * STEP]);
                    changed = true;
                }
                if enter {
                    state.grab = None;
                }
            }
        } else {
            if across != 0.0 {
                state.selected = (state.selected as isize + across as isize)
                    .clamp(0, count as isize - 1) as usize;
            }
            if enter {
                state.grab = Some(curve.points()[state.selected]);
            }
        }
        // Left and right are always the graph's; up, down and back only while a point is
        // grabbed (else up and down leave it like any row). Egui applies this to the next
        // pass's keys, so it's set after this pass's have grabbed or let go.
        let grabbed = state.grab.is_some();
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: grabbed,
                    escape: grabbed,
                    ..Default::default()
                },
            );
        });
    }
    // The mouse.
    let pointer = response.interact_pointer_pos();
    if response.drag_started()
        && let Some(i) = pointer.and_then(|at| nearest(curve, at))
    {
        state.selected = i;
        state.dragging = true;
        state.grab = None;
    }
    if state.dragging
        && response.dragged()
        && let Some(at) = pointer
    {
        curve.set_point(state.selected, from_screen(at));
        changed = true;
    }
    if response.drag_stopped() {
        state.dragging = false;
    }
    if let Some(at) = pointer {
        if response.double_clicked() {
            if nearest(curve, at).is_none()
                && let Some(i) = curve.insert(from_screen(at))
            {
                state.selected = i;
                changed = true;
            }
        } else if response.clicked() && !enter {
            if let Some(i) = nearest(curve, at) {
                state.selected = i;
            }
        } else if response.secondary_clicked()
            && let Some(i) = nearest(curve, at)
            && curve.remove(i)
        {
            state.selected = i - 1;
            changed = true;
        }
    }
    let count = curve.points().len();
    state.selected = state.selected.min(count - 1);

    // The graph.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 7.0, if focused { ROW_FOCUS } else { ROW_FILL });
    painter.rect_filled(plot, 3.0, Color32::from_gray(20));
    let grid = Stroke::new(1.0, Color32::from_gray(48));
    for k in 1..4 {
        let f = k as f32 / 4.0;
        painter.line_segment([to_screen([f, 0.0]), to_screen([f, 1.0])], grid);
        painter.line_segment([to_screen([0.0, f]), to_screen([1.0, f])], grid);
    }
    painter.extend(Shape::dashed_line(
        &[to_screen([0.0, 0.0]), to_screen([1.0, 1.0])],
        Stroke::new(1.0, Color32::from_gray(80)),
        4.0,
        4.0,
    ));
    let small = FontId::proportional(12.0);
    painter.text(
        plot.left_bottom() + vec2(0.0, 4.0),
        Align2::LEFT_TOP,
        "0",
        small.clone(),
        WEAK,
    );
    painter.text(
        plot.right_bottom() + vec2(0.0, 4.0),
        Align2::RIGHT_TOP,
        "Pushed 100 %",
        small.clone(),
        WEAK,
    );
    painter.text(
        plot.left_top() - vec2(6.0, 0.0),
        Align2::RIGHT_TOP,
        "100",
        small.clone(),
        WEAK,
    );
    painter.text(
        plot.left_top() - vec2(0.0, 4.0),
        Align2::LEFT_BOTTOM,
        "Asks for",
        small.clone(),
        WEAK,
    );
    // Where the control is now.
    if let Some(push) = live {
        let x = push.abs().min(1.0);
        let on = to_screen([x, curve.at(x)]);
        let faint = Stroke::new(1.0, ON.gamma_multiply(0.6));
        painter.line_segment([to_screen([x, 0.0]), on], faint);
        painter.line_segment([to_screen([0.0, curve.at(x)]), on], faint);
        painter.circle_filled(on, 4.5, ON);
    }
    let line: Vec<Pos2> = (0..=120)
        .map(|k| {
            let x = k as f32 / 120.0;
            to_screen([x, curve.at(x)])
        })
        .collect();
    painter.add(Shape::line(line, Stroke::new(2.5, ACCENT)));
    for (i, &p) in curve.points().iter().enumerate() {
        let at = to_screen(p);
        if i == state.selected {
            if state.grab.is_some() || state.dragging {
                painter.circle_filled(at, 7.0, ACCENT);
                painter.circle_stroke(at, 9.0, Stroke::new(2.0, Color32::WHITE));
            } else {
                painter.circle_filled(at, 5.5, Color32::WHITE);
                painter.circle_stroke(at, 9.0, Stroke::new(2.0, ACCENT));
            }
        } else {
            painter.circle_filled(at, 4.5, Color32::from_gray(200));
        }
    }
    // The picked point, and what the buttons do.
    let [x, y] = curve.points()[state.selected];
    let column = pos2(plot.right() + 22.0, plot.top());
    let lines = [
        (format!("Point {} of {count}", state.selected + 1), TEXT),
        (format!("Pushed {:.0} %", x * 100.0), WEAK),
        (format!("Asks for {:.0} %", y * 100.0), WEAK),
    ];
    for (k, (text, color)) in lines.into_iter().enumerate() {
        painter.text(
            column + vec2(0.0, k as f32 * 20.0),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(15.0),
            color,
        );
    }
    if let Some(push) = live {
        painter.text(
            column + vec2(0.0, 72.0),
            Align2::LEFT_TOP,
            format!(
                "Now {:.0} % → {:.0} %",
                push.abs() * 100.0,
                curve.at(push).abs() * 100.0
            ),
            FontId::proportional(14.0),
            ON,
        );
    }
    let help = match (state.grab.is_some(), kit::device(ui.ctx())) {
        (_, Device::Mouse) => "Drag a point.\nDouble-click adds one,\nright-click removes one.",
        (true, _) => "Arrows move it.\nChoose keeps it,\nback puts it back.",
        (false, _) => "Left and right\npick a point,\nchoose grabs it.",
    };
    painter.text(
        pos2(column.x, plot.bottom()),
        Align2::LEFT_BOTTOM,
        help,
        FontId::proportional(13.0),
        WEAK,
    );

    // Presets, adding and removing points.
    let preset = Preset::of(curve);
    let mut names: Vec<String> = Preset::ALL.iter().map(|p| p.name().to_string()).collect();
    names.push("Your own".into());
    let mut index = preset.map_or(Preset::ALL.len(), |p| {
        Preset::ALL.iter().position(|&q| q == p).unwrap_or(0)
    });
    if kit::choice(ui, "Preset", &mut index, &names).changed()
        && let Some(preset) = Preset::ALL.get(index)
    {
        *curve = preset.curve();
        state.selected = 0;
        changed = true;
    }
    let full = count >= crate::input::curve::MOST_POINTS;
    if ui
        .add_enabled_ui(!full, |ui| kit::button(ui, "Add a point"))
        .inner
        .clicked()
        && let Some(i) = curve.split_widest()
    {
        state.selected = i;
        changed = true;
    }
    let end = state.selected == 0 || state.selected + 1 >= count;
    let label = format!("Remove point {}", state.selected + 1);
    if ui
        .add_enabled_ui(!end, |ui| kit::button(ui, &label))
        .inner
        .clicked()
        && curve.remove(state.selected)
    {
        state.selected -= 1;
        changed = true;
    }

    let grabbed = state.grab.is_some();
    ui.data_mut(|d| d.insert_temp(state_id, state));
    Edited { changed, grabbed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::kit::Panel;
    use std::cell::{Cell, RefCell};

    #[test]
    fn points_are_picked_grabbed_and_moved_with_the_buttons() {
        let ctx = egui::Context::default();
        crate::menu::style(&ctx);
        crate::menu::nav::hook(&ctx);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let curve = RefCell::new(Preset::Stick.curve());
        let (cancel, grabbed) = (Cell::new(false), Cell::new(false));
        let pass = |keys: &[Key]| {
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
                Panel::new("test", "Test").show(root.ctx(), |ui| {
                    let edited = editor(ui, "test", &mut curve.borrow_mut(), None, cancel.take());
                    grabbed.set(edited.grabbed);
                });
            });
        };
        let point = |i: usize| curve.borrow().points()[i];
        pass(&[]);
        pass(&[]);
        // Right picks the second point, choose grabs it, up twice lifts it, choose keeps.
        let was = point(1);
        pass(&[Key::ArrowRight]);
        pass(&[Key::Enter]);
        assert!(grabbed.get());
        pass(&[Key::ArrowUp]);
        pass(&[Key::ArrowUp]);
        pass(&[Key::Enter]);
        assert!(!grabbed.get());
        assert!((point(1)[1] - (was[1] + 2.0 * STEP)).abs() < 1e-5);
        assert_eq!(point(1)[0], was[0]);
        // Grabbed again and moved, back puts it back.
        let kept = point(1);
        pass(&[Key::Enter]);
        pass(&[Key::ArrowRight]);
        assert_ne!(point(1), kept);
        cancel.set(true);
        pass(&[]);
        assert_eq!(point(1), kept);
        assert!(!grabbed.get());
        // Not grabbed, down leaves the graph for the preset ("Your own", the last): left
        // picks the last preset.
        pass(&[Key::ArrowDown]);
        pass(&[Key::ArrowLeft]);
        assert_eq!(*curve.borrow(), Preset::Quick.curve());
    }
}
