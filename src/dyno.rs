//! An engine dyno (5): the active car's engine on a brake that holds its speed, swept at
//! full throttle from low rpm up to the rev limiter, plotting torque and power at the
//! crank. It runs a copy of the engine (as built, tuned and worn), so a run doesn't wear
//! the car's; the last few runs stay on the plot to compare.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};
use crate::physics::engine::{Engine, EngineBuild};
use crate::ui::{ToolInput, tool_window};

/// The dyno's physics step (s), and the simulated time run per frame (s).
const STEP: f32 = 1.0 / 120.0;
const PER_FRAME: f32 = 1.0;
/// Where the sweep starts (rpm), how long it holds there first (s), how far below the
/// rev limiter it ends (rpm), and how often it samples (rpm).
const START_RPM: f32 = 1000.0;
const SETTLE: f32 = 3.0;
const END_MARGIN: f32 = 50.0;
const SAMPLE_RPM: f32 = 100.0;
/// Air the dyno cell's fan blows through the radiator (m/s).
const FAN_AIRSPEED: f32 = 20.0;
/// Watts per mechanical horsepower.
const HORSEPOWER: f32 = 745.7;
/// Runs kept on the plot, newest first, and their colours.
const KEPT: usize = 4;
const COLORS: [Color32; KEPT] = [
    Color32::from_rgb(255, 196, 60),
    Color32::from_rgb(90, 170, 255),
    Color32::from_rgb(120, 220, 120),
    Color32::from_rgb(230, 110, 200),
];

pub struct DynoPlugin;

impl Plugin for DynoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dyno>()
            .add_systems(Update, run)
            .add_systems(EguiPrimaryContextPass, window);
    }
}

#[derive(Resource)]
struct Dyno {
    expanded: bool,
    /// How fast the sweep rises (rpm/s): slower is closer to steady state (a turbo has
    /// time to spool), faster is like a quick pull.
    rate: f32,
    sweep: Option<Sweep>,
    /// Finished runs, newest first.
    runs: Vec<Curve>,
}

impl Default for Dyno {
    fn default() -> Self {
        Self {
            expanded: false,
            rate: 250.0,
            sweep: None,
            runs: Vec::new(),
        }
    }
}

/// A run's torque at the crank by rpm, and what it was run on.
#[derive(Clone)]
struct Curve {
    pub label: String,
    /// (rpm, N·m)
    pub points: Vec<(f32, f32)>,
    /// What broke during the run, if anything.
    pub failures: Vec<String>,
}

impl Curve {
    /// Power (W) at a point.
    fn power(rpm: f32, torque: f32) -> f32 {
        torque * rpm * std::f32::consts::TAU / 60.0
    }

    /// The highest torque and power, with their rpm.
    fn peaks(&self) -> Option<((f32, f32), (f32, f32))> {
        let torque = self
            .points
            .iter()
            .copied()
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        let power = self
            .points
            .iter()
            .map(|&(rpm, torque)| (rpm, Self::power(rpm, torque)))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        Some((torque, power))
    }
}

/// A sweep in progress: the engine held at `rpm` at full throttle, the speed rising at a
/// steady rate after settling at the start.
struct Sweep {
    build: EngineBuild,
    engine: Engine,
    rate: f32,
    rpm: f32,
    end: f32,
    settle: f32,
    next_sample: f32,
    curve: Curve,
}

impl Sweep {
    pub fn new(build: &EngineBuild, engine: &Engine, rate: f32) -> Self {
        let mut engine = *engine;
        engine.airspeed = FAN_AIRSPEED;
        let label = format!("{}, {:.0} octane", build.hardware.name, build.octane);
        Self {
            end: build.ecu.rev_limit - END_MARGIN,
            build: build.clone(),
            engine,
            rate,
            rpm: START_RPM,
            settle: SETTLE,
            next_sample: START_RPM,
            curve: Curve {
                label,
                points: Vec::new(),
                failures: Vec::new(),
            },
        }
    }

    /// Share of the sweep done (0..1).
    fn progress(&self) -> f32 {
        ((self.rpm - START_RPM) / (self.end - START_RPM)).clamp(0.0, 1.0)
    }

    /// Runs the dyno for `seconds`; the finished curve once the sweep reaches its end (or
    /// the engine breaks).
    pub fn advance(&mut self, seconds: f32) -> Option<Curve> {
        let (build, engine) = (&self.build, &mut self.engine);
        for _ in 0..(seconds / STEP).round() as usize {
            engine.step(build, 1.0, self.rpm, true, STEP);
            if self.settle > 0.0 {
                self.settle -= STEP;
                continue;
            }
            if self.rpm >= self.next_sample {
                let torque = engine.torque(build, self.rpm);
                self.curve.points.push((self.rpm, torque));
                self.next_sample += SAMPLE_RPM;
            }
            let blown = engine.blown(&build.hardware);
            if self.rpm >= self.end || blown {
                let cylinders = build.hardware.cylinders as usize;
                self.curve.failures = engine.damage.failures(cylinders);
                return Some(self.curve.clone());
            }
            self.rpm = (self.rpm + self.rate * STEP).min(self.end);
        }
        None
    }
}

fn run(mut dyno: ResMut<Dyno>) {
    let Some(sweep) = &mut dyno.sweep else {
        return;
    };
    if let Some(curve) = sweep.advance(PER_FRAME) {
        dyno.sweep = None;
        dyno.runs.insert(0, curve);
        dyno.runs.truncate(KEPT);
    }
}

fn window(
    mut contexts: EguiContexts,
    input: ToolInput,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut dyno: ResMut<Dyno>,
) -> Result {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let tool = ("Engine dyno", KeyCode::Digit5);
    let Some((window, _)) = tool_window(ctx, tool, 2, &input, &mut dyno.expanded) else {
        return Ok(());
    };
    window.default_width(620.0).show(ctx, |ui| {
        controls(ui, &mut dyno, car);
        ui.separator();
        plot(ui, &dyno);
        ui.separator();
        legend(ui, &dyno);
    });
    Ok(())
}

/// Starting and stopping a run, and the sweep rate.
fn controls(ui: &mut egui::Ui, dyno: &mut Dyno, car: &Car) {
    ui.horizontal(|ui| {
        if let Some(sweep) = &dyno.sweep {
            ui.add(
                egui::ProgressBar::new(sweep.progress())
                    .desired_width(200.0)
                    .text(format!("{:.0} rpm", sweep.rpm)),
            );
            if ui.button("Stop").clicked() {
                dyno.sweep = None;
            }
        } else {
            let build = &car.setup.engine.build;
            if ui.button("Run").clicked() {
                let engine = &car.drivetrain.engine;
                dyno.sweep = Some(Sweep::new(build, engine, dyno.rate));
            }
            ui.add(
                egui::Slider::new(&mut dyno.rate, 100.0..=1000.0)
                    .step_by(50.0)
                    .suffix(" rpm/s")
                    .text("sweep"),
            )
            .on_hover_text(
                "How fast the revs rise: slowly, a turbo has time to spool (close to steady \
                 state)",
            );
            if ui
                .add_enabled(!dyno.runs.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                dyno.runs.clear();
            }
        }
    });
    ui.label(format!(
        "{} at the crank, full throttle from {START_RPM:.0} rpm to the rev limiter \
         ({:.0} rpm)",
        car.setup.engine.build.hardware.name, car.setup.engine.build.ecu.rev_limit
    ));
}

/// Torque (solid) and power (dashed) of the runs against rpm, the newest on top, with its
/// readings under the pointer.
fn plot(ui: &mut egui::Ui, dyno: &Dyno) {
    let live = dyno.sweep.as_ref().map(|sweep| &sweep.curve);
    let curves = live.into_iter().chain(dyno.runs.iter());
    let mut lines = Vec::new();
    for (k, (curve, color)) in curves.zip(COLORS).enumerate() {
        let newest = k == 0;
        lines.push(Line {
            points: curve.points.clone(),
            axis: Axis::Torque,
            color,
            dashed: false,
            name: newest.then_some(""),
        });
        lines.push(Line {
            points: (curve.points.iter())
                .map(|&(rpm, torque)| (rpm, horsepower(rpm, torque.max(0.0))))
                .collect(),
            axis: Axis::Power,
            color,
            dashed: true,
            name: newest.then_some(""),
        });
    }
    chart(ui, &lines, 320.0);
}

/// Which scale a line on a chart is read against: torque on the left, power on the right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
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
struct Line<'a> {
    pub points: Vec<(f32, f32)>,
    pub axis: Axis,
    pub color: Color32,
    pub dashed: bool,
    pub name: Option<&'a str>,
}

/// Power (hp) at `rpm` with `torque` (N·m).
fn horsepower(rpm: f32, torque: f32) -> f32 {
    Curve::power(rpm, torque) / HORSEPOWER
}

/// Rounds `value` up to a whole number of `step`s.
fn ceil_to(value: f32, step: f32) -> f32 {
    (value / step).ceil() * step
}

/// Torque and power against rpm, `height` high: the first line on top, with the named
/// lines' readings under the pointer.
fn chart(ui: &mut egui::Ui, lines: &[Line], height: f32) {
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

/// Each run's peaks, in its colour.
fn legend(ui: &mut egui::Ui, dyno: &Dyno) {
    ui.label("Solid: torque (N·m, left). Dashed: power (hp, right).");
    let live = dyno.sweep.is_some() as usize;
    for (curve, color) in dyno.runs.iter().zip(COLORS.iter().skip(live)) {
        let Some(((torque_rpm, torque), (power_rpm, power))) = curve.peaks() else {
            continue;
        };
        let mut line = format!(
            "{}: {torque:.0} N·m at {torque_rpm:.0} rpm, {:.0} hp ({:.0} kW) at \
             {power_rpm:.0} rpm",
            curve.label,
            power / HORSEPOWER,
            power / 1000.0,
        );
        if !curve.failures.is_empty() {
            line += &format!(" (broke: {})", curve.failures.join(", "));
        }
        ui.colored_label(*color, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::hardware::EngineHardware;

    fn sweep(build: &EngineBuild, rate: f32) -> Curve {
        let mut sweep = Sweep::new(build, &Engine::default(), rate);
        loop {
            if let Some(curve) = sweep.advance(PER_FRAME) {
                return curve;
            }
        }
    }

    #[test]
    fn sweeps_the_stock_engines() {
        let na = sweep(&EngineBuild::default(), 250.0);
        let ((torque_rpm, torque), (power_rpm, power)) = na.peaks().unwrap();
        assert!((180.0..230.0).contains(&torque), "{torque} N·m");
        assert!((3000.0..5500.0).contains(&torque_rpm), "at {torque_rpm}");
        assert!(power_rpm > torque_rpm, "power peaks at {power_rpm}");
        assert!((100e3..160e3).contains(&power), "{power} W");
        assert_eq!(na.points.first().unwrap().0, START_RPM);
        assert!(na.points.last().unwrap().0 > 6600.0);
        assert!(na.failures.is_empty());
        // The turbo makes more, and a quick pull shows less of it low down, before the
        // turbo has spooled.
        let turbo = EngineBuild::new(
            EngineHardware::turbo(),
            crate::physics::engine::ecu::EcuModel::Stock,
        );
        let slow = sweep(&turbo, 150.0);
        assert!(slow.peaks().unwrap().0.1 > torque * 1.3);
        let quick = sweep(&turbo, 1000.0);
        let at = |curve: &Curve, rpm: f32| curve.points.iter().find(|p| p.0 >= rpm).unwrap().1;
        assert!(
            at(&quick, 3000.0) < at(&slow, 3000.0),
            "{} vs {}",
            at(&quick, 3000.0),
            at(&slow, 3000.0)
        );
    }
}
