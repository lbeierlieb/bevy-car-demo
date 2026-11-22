//! A chart of torque and power by rpm, as the dyno's results show it.

use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2};

/// Watts in a (mechanical) horsepower.
pub const HORSEPOWER: f32 = 745.7;

/// Which scale a line on a chart is read against: torque on the left, power on the right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Torque,
    Power,
}

impl Axis {
    fn unit(self) -> &'static str {
        match self {
            Axis::Torque => "N·m",
            Axis::Power => "hp",
        }
    }
}

/// A line on a chart: values (N·m or hp, by `axis`) by rpm. Lines with a `name` show their
/// value under the pointer, named so.
pub struct Line<'a> {
    pub points: Vec<(f32, f32)>,
    pub axis: Axis,
    pub color: Color32,
    pub dashed: bool,
    pub name: Option<&'a str>,
}

/// Rounds `value` up to a whole number of `step`s.
fn ceil_to(value: f32, step: f32) -> f32 {
    (value / step).ceil() * step
}

/// Torque and power against rpm, `height` high: the first line on top, with the named
/// lines' readings under the pointer.
pub fn chart(ui: &mut egui::Ui, lines: &[Line], height: f32) {
    let width = ui.available_width().max(400.0);
    let (response, painter) = ui.allocate_painter(egui::vec2(width, height), Sense::hover());
    let frame = response.rect;
    let area = Rect::from_min_max(
        frame.min + egui::vec2(48.0, 10.0),
        frame.max - egui::vec2(48.0, 24.0),
    );
    let text = ui.visuals().text_color();
    let grid = ui.visuals().widgets.noninteractive.bg_stroke.color;

    let (mut top_rpm, mut top_torque, mut top_power) = (7000.0f32, 100.0f32, 100.0f32);
    for line in lines {
        for &(rpm, value) in &line.points {
            top_rpm = top_rpm.max(rpm);
            match line.axis {
                Axis::Torque => top_torque = top_torque.max(value),
                Axis::Power => top_power = top_power.max(value),
            }
        }
    }
    let top_rpm = ceil_to(top_rpm, 1000.0);
    let top_torque = ceil_to(top_torque * 1.05, 50.0);
    let top_power = ceil_to(top_power * 1.05, 50.0);
    let top = |axis: Axis| match axis {
        Axis::Torque => top_torque,
        Axis::Power => top_power,
    };
    let x = |rpm: f32| egui::lerp(area.left()..=area.right(), rpm / top_rpm);
    let y = |value: f32, top: f32| egui::lerp(area.bottom()..=area.top(), value / top);

    // Grid: rpm every thousand, torque on the left and power on the right in fifths.
    let font = FontId::proportional(11.0);
    for i in 0..=(top_rpm / 1000.0) as usize {
        let rpm = i as f32 * 1000.0;
        painter.vline(x(rpm), area.y_range(), Stroke::new(1.0, grid));
        let at = pos2(x(rpm), area.bottom() + 4.0);
        painter.text(at, Align2::CENTER_TOP, format!("{}", i), font.clone(), text);
    }
    for i in 0..=5 {
        let share = i as f32 / 5.0;
        let at = y(share, 1.0);
        painter.hline(area.x_range(), at, Stroke::new(1.0, grid));
        let torque = format!("{:.0}", share * top_torque);
        painter.text(
            pos2(area.left() - 4.0, at),
            Align2::RIGHT_CENTER,
            torque,
            font.clone(),
            text,
        );
        let power = format!("{:.0}", share * top_power);
        painter.text(
            pos2(area.right() + 4.0, at),
            Align2::LEFT_CENTER,
            power,
            font.clone(),
            text,
        );
    }
    painter.text(
        frame.left_top(),
        Align2::LEFT_TOP,
        "N·m",
        font.clone(),
        text,
    );
    painter.text(
        frame.right_top(),
        Align2::RIGHT_TOP,
        "hp",
        font.clone(),
        text,
    );
    let label = pos2(area.center().x, frame.bottom());
    painter.text(
        label,
        Align2::CENTER_BOTTOM,
        "rpm × 1000",
        font.clone(),
        text,
    );

    for line in lines.iter().rev() {
        let points: Vec<Pos2> = line
            .points
            .iter()
            .map(|&(rpm, value)| pos2(x(rpm), y(value.max(0.0), top(line.axis))))
            .collect();
        let stroke = Stroke::new(2.0, line.color);
        if line.dashed {
            painter.extend(Shape::dashed_line(&points, stroke, 6.0, 4.0));
        } else {
            painter.add(Shape::line(points, stroke));
        }
    }

    // Readings of the named lines under the pointer.
    let Some(pointer) = response.hover_pos().filter(|p| area.contains(*p)) else {
        return;
    };
    let rpm = (pointer.x - area.left()) / area.width() * top_rpm;
    let mut place = pos2(x(rpm) + 6.0, area.top() + 4.0);
    let mut shown = false;
    for line in lines {
        let Some(name) = line.name else {
            continue;
        };
        let Some(&(at, value)) = line
            .points
            .iter()
            .min_by(|a, b| (a.0 - rpm).abs().total_cmp(&(b.0 - rpm).abs()))
        else {
            continue;
        };
        if !shown {
            painter.vline(x(at), area.y_range(), Stroke::new(1.0, text));
            let reading = format!("{at:.0} rpm");
            painter.text(place, Align2::LEFT_TOP, reading, font.clone(), text);
            place.y += 14.0;
            shown = true;
        }
        let reading = format!("{name}{value:.0} {}", line.axis.unit());
        painter.text(place, Align2::LEFT_TOP, reading, font.clone(), line.color);
        place.y += 14.0;
    }
}
