//! Driving HUD in the bottom right corner: speed and gear in the middle, with a rev counter
//! arc around them, and the tires' temperatures and wear, the coolant
//! temperature and the fuel left of it.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Shape, Stroke, pos2, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car, Rescued};
use crate::menu::MEDIUM;
use crate::physics::engine::damage::COOLANT_LIMIT;
use crate::physics::tire::{AMBIENT, Compound, RIBS};

/// Gauge radius (to the middle of the rev arc) and the arc's sweep: 270°, open at the
/// bottom, from 0 rpm at the lower left clockwise to the top of the scale at the lower right.
const RADIUS: f32 = 78.0;
const START: f32 = 0.75 * PI;
const SWEEP: f32 = 1.5 * PI;

const BACKGROUND: Color32 = Color32::from_rgba_premultiplied(10, 12, 16, 190);
const TRACK: Color32 = Color32::from_rgba_premultiplied(70, 74, 82, 255);
const BAR: Color32 = Color32::from_rgb(240, 240, 235);
const RED: Color32 = Color32::from_rgb(230, 50, 40);
const ORANGE: Color32 = Color32::from_rgb(255, 150, 50);
const DIM: Color32 = Color32::from_rgb(150, 154, 162);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, (hud, rescue_warning));
    }
}

/// How slowly the gauges' needles follow: the time constants (s) of the fuel gauge (slow,
/// like a real one, which also hides the fuel sloshing) and the coolant gauge.
const FUEL_LAG: f32 = 2.5;
const COOLANT_LAG: f32 = 1.2;

/// What the fuel and coolant gauges show: their needles swing towards the car's values
/// with a lag, and start where they belong for a car shown afresh.
#[derive(Default)]
struct Needles {
    car: Option<Entity>,
    fuel: f32,
    coolant: f32,
}

impl Needles {
    fn follow(&mut self, entity: Entity, car: &Car, dt: f32) {
        let fuel = (car.fuel.litres() / car.setup.tank.litres).clamp(0.0, 1.0);
        let coolant = car.drivetrain.engine.thermal.coolant;
        if self.car != Some(entity) {
            *self = Self {
                car: Some(entity),
                fuel,
                coolant,
            };
        }
        self.fuel += (fuel - self.fuel) * (1.0 - (-dt / FUEL_LAG).exp());
        self.coolant += (coolant - self.coolant) * (1.0 - (-dt / COOLANT_LAG).exp());
    }
}

/// While the rescue helicopter has the car (`car::Rescued`: from the call until it's set
/// down), a warning in the upper middle of the screen, its border pulsing.
fn rescue_warning(
    mut contexts: EguiContexts,
    rescued: Option<Res<Rescued>>,
    time: Res<Time>,
) -> Result {
    if !rescued.is_some_and(|rescued| rescued.0) {
        return Ok(());
    }
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 5.0).sin();
    let border = Color32::from_rgb(255, (150.0 + 70.0 * pulse) as u8, 30);
    egui::Area::new(egui::Id::new("rescue warning"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, 150.0))
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(150, 20, 15, 225))
                .stroke(Stroke::new(3.0 + 2.0 * pulse, border))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(24, 14))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("⚠ You are being rescued ⚠")
                                .font(FontId::new(30.0, egui::FontFamily::Name(MEDIUM.into())))
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            egui::RichText::new("Please do not resist!")
                                .font(FontId::new(22.0, egui::FontFamily::Name(MEDIUM.into())))
                                .color(Color32::from_rgb(255, 225, 120)),
                        );
                    });
                });
        });
    Ok(())
}

fn hud(
    mut contexts: EguiContexts,
    active: Res<ActiveCar>,
    cars: Query<(Entity, &Car)>,
    time: Res<Time>,
    mut needles: Local<Needles>,
) -> Result {
    let Some((entity, car)) = cars.iter().find(|(_, car)| car.index == active.0) else {
        return Ok(());
    };
    needles.follow(entity, car, time.delta_secs());
    let engine = &car.setup.engine;
    let rpm = car.drivetrain.rpm;
    let top = engine.rev_scale();
    let gear = match car.drivetrain.selected_gear() {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        g => g.to_string(),
    };

    egui::Area::new(egui::Id::new("hud"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -16.0))
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            let size = 2.0 * (RADIUS + 18.0);
            let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
            let painter = ui.painter();
            let center = rect.center();
            let at = |rpm: f32, radius: f32| {
                let angle = START + SWEEP * (rpm / top).clamp(0.0, 1.0);
                center + radius * vec2(angle.cos(), angle.sin())
            };
            let arc = |from: f32, to: f32, radius: f32, width: f32, color: Color32| {
                let steps = ((to - from) / top * 96.0).ceil().max(1.0) as usize;
                let points = (0..=steps)
                    .map(|i| at(from + (to - from) * i as f32 / steps as f32, radius))
                    .collect::<Vec<Pos2>>();
                Shape::line(points, Stroke::new(width, color))
            };

            painter.circle_filled(center, RADIUS + 16.0, BACKGROUND);
            painter.add(arc(0.0, top, RADIUS, 10.0, TRACK));
            painter.add(arc(engine.redline_rpm(), top, RADIUS + 7.0, 3.0, RED));
            let bar = if rpm >= engine.redline_rpm() {
                RED
            } else {
                BAR
            };
            painter.add(arc(0.0, rpm.min(top), RADIUS, 10.0, bar));
            // A tick and a number every thousand rpm.
            for k in 0..=(top / 1000.0) as usize {
                let r = k as f32 * 1000.0;
                let color = if r >= engine.redline_rpm() { RED } else { DIM };
                painter.line_segment([at(r, RADIUS - 8.0), at(r, RADIUS - 14.0)], (2.0, color));
                painter.text(
                    at(r, RADIUS - 23.0),
                    Align2::CENTER_CENTER,
                    k.to_string(),
                    FontId::proportional(11.0),
                    color,
                );
            }

            // Speed above gear in the middle, from the gearbox output like a real one.
            let speed = (car.shaft_speed().abs() * 3.6).round();
            painter.text(
                center + vec2(0.0, -14.0),
                Align2::CENTER_CENTER,
                format!("{speed:.0}"),
                FontId::proportional(34.0),
                Color32::WHITE,
            );
            painter.text(
                center + vec2(0.0, 8.0),
                Align2::CENTER_CENTER,
                "km/h",
                FontId::proportional(11.0),
                DIM,
            );
            painter.text(
                center + vec2(0.0, 32.0),
                Align2::CENTER_CENTER,
                gear,
                FontId::proportional(26.0),
                Color32::from_rgb(255, 196, 60),
            );
            // The parking brake's warning lamp, under the gear.
            if car.parking_brake {
                painter.text(
                    center + vec2(0.0, 56.0),
                    Align2::CENTER_CENTER,
                    "(P)",
                    FontId::proportional(15.0),
                    RED,
                );
            }
            let cylinders = engine.build.hardware.cylinders as usize;
            let failure = car.drivetrain.engine.damage.failures(cylinders).pop();
            if !car.drivetrain.running {
                let blown = car.drivetrain.engine.blown(&engine.build.hardware);
                painter.text(
                    center + vec2(0.0, RADIUS + 4.0),
                    Align2::CENTER_CENTER,
                    if blown {
                        "ENGINE BLOWN (R)"
                    } else if car.drivetrain.cranking() {
                        "STARTING"
                    } else if car.drivetrain.switched_off() {
                        "ENGINE OFF (hold I)"
                    } else {
                        "STALLED (hold I)"
                    },
                    FontId::proportional(10.0),
                    RED,
                );
            } else if let Some(failure) = failure {
                painter.text(
                    center + vec2(0.0, RADIUS + 4.0),
                    Align2::CENTER_CENTER,
                    failure.to_uppercase(),
                    FontId::proportional(10.0),
                    ORANGE,
                );
            }
        });

    tires(contexts.ctx_mut()?, car, &needles);
    Ok(())
}

/// Tire size in the tire display, and the gap between them.
const TIRE: egui::Vec2 = vec2(30.0, 34.0);
const TIRE_GAP: egui::Vec2 = vec2(14.0, 22.0);
/// Room under each tire for its pressure.
const PRESSURE_ROW: f32 = 14.0;
/// The gauges right of the tires (coolant, then fuel): the gap to the tires, each one's
/// column (labels left of the bar), the bar's width, the room for the symbol on top, and
/// the gap between a bar's segments.
const GAUGES_GAP: f32 = 10.0;
const COLUMN: f32 = 22.0;
const BAR_WIDTH: f32 = 8.0;
const SYMBOL: f32 = 22.0;
const SEGMENT_GAP: f32 = 2.0;
/// Below this share of the tank the fuel gauge turns orange.
const FUEL_LOW: f32 = 0.15;
/// The coolant gauge's scale (K, 40 to 130 °C): blue below `COOLANT_COLD`, orange from
/// `COOLANT_HOT`, red from where the engine takes damage.
const COOLANT_SCALE: (f32, f32) = (313.15, 403.15);
const COOLANT_COLD: f32 = 333.15;
const COOLANT_HOT: f32 = 385.15;
const BLUE: Color32 = Color32::from_rgb(90, 150, 240);

/// The four tires left of the gauge, seen from above: each rib (inside towards the car's
/// middle) coloured by how warm its surface is against the compound's window (blue cold,
/// green right, red overheated) with its worn share darkened from the top, the core
/// temperature in the middle, and the pressure below. Right of them, the coolant and fuel
/// gauges (see `coolant_gauge`, `fuel_gauge`).
fn tires(ctx: &egui::Context, car: &Car, needles: &Needles) {
    let size = 2.0 * TIRE + TIRE_GAP + vec2(16.0 + GAUGES_GAP + 2.0 * COLUMN, 16.0 + PRESSURE_ROW);
    egui::Area::new(egui::Id::new("tires"))
        .anchor(
            Align2::RIGHT_BOTTOM,
            vec2(-16.0 - 2.0 * (RADIUS + 18.0) - 10.0, -16.0),
        )
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 8.0, BACKGROUND);
            for (i, corner) in car.corners.iter().enumerate() {
                let (column, row) = ((i % 2) as f32, (i / 2) as f32);
                let min = rect.min + vec2(8.0, 8.0) + vec2(column, row) * (TIRE + TIRE_GAP);
                let tire = egui::Rect::from_min_size(min, TIRE);
                let params = &car.setup.axle(corner.front).tire;
                let tread = corner.tread;
                let rib_width = TIRE.x / RIBS as f32;
                for rib in 0..RIBS {
                    // The inside of a left tire is on its right.
                    let place = if corner.side < 0.0 {
                        RIBS - 1 - rib
                    } else {
                        rib
                    };
                    let x = min.x + place as f32 * rib_width;
                    let strip = egui::Rect::from_min_size(pos2(x, min.y), vec2(rib_width, TIRE.y));
                    let warmth = warmth_color(&params.compound, tread.surface[rib]);
                    painter.rect_filled(strip, 1.0, warmth);
                    let worn = vec2(rib_width, TIRE.y * tread.wear[rib]);
                    let worn = egui::Rect::from_min_size(strip.min, worn);
                    painter.rect_filled(worn, 1.0, Color32::from_black_alpha(150));
                }
                painter.rect_stroke(
                    tire,
                    3.0,
                    egui::Stroke::new(1.5, BACKGROUND),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    tire.center(),
                    Align2::CENTER_CENTER,
                    format!("{:.0}", tread.core_temperature()),
                    FontId::proportional(11.0),
                    Color32::BLACK,
                );
                painter.text(
                    pos2(tire.center().x, tire.max.y + 2.0),
                    Align2::CENTER_TOP,
                    format!("{:.2}", params.pressure(&tread)),
                    FontId::proportional(10.0),
                    Color32::WHITE,
                );
            }

            let column = |n: f32| {
                let right = rect.right() - 8.0 - n * COLUMN;
                egui::Rect::from_min_max(
                    pos2(right - COLUMN, rect.top() + 8.0),
                    pos2(right, rect.bottom() - 8.0),
                )
            };
            coolant_gauge(painter, column(1.0), needles.coolant);
            fuel_gauge(painter, column(0.0), needles.fuel, car);
        });
}

/// A gauge's column split into the symbol's place on top and the bar below it, with
/// the labels at the bar's ends left of it.
fn gauge_layout(
    painter: &egui::Painter,
    column: egui::Rect,
    labels: (&str, &str),
) -> (Pos2, egui::Rect) {
    let symbol = pos2(column.center().x, column.top() + SYMBOL / 2.0 - 2.0);
    let bar = egui::Rect::from_min_max(
        pos2(column.right() - 3.0 - BAR_WIDTH, column.top() + SYMBOL),
        pos2(column.right() - 3.0, column.bottom()),
    );
    let x = bar.left() - 6.0;
    let font = FontId::proportional(9.0);
    let (top, bottom) = labels;
    painter.text(
        pos2(x, bar.top()),
        Align2::CENTER_TOP,
        top,
        font.clone(),
        DIM,
    );
    painter.text(
        pos2(x, bar.bottom()),
        Align2::CENTER_BOTTOM,
        bottom,
        font,
        DIM,
    );
    (symbol, bar)
}

/// The fuel left (`share`, as the needle shows it) as four segments from E to F under a fuel pump, the pump lit orange
/// when low and red while the engine starves (so is the bar).
fn fuel_gauge(painter: &egui::Painter, column: egui::Rect, share: f32, car: &Car) {
    let (symbol, bar) = gauge_layout(painter, column, ("F", "E"));
    let (color, light) = if car.fuel.supply < 0.95 {
        (RED, RED)
    } else if share < FUEL_LOW {
        (ORANGE, ORANGE)
    } else {
        (BAR, DIM)
    };
    let height = (bar.height() - 3.0 * SEGMENT_GAP) / 4.0;
    for k in 0..4 {
        let bottom = bar.bottom() - k as f32 * (height + SEGMENT_GAP);
        let segment =
            egui::Rect::from_min_max(pos2(bar.left(), bottom - height), pos2(bar.right(), bottom));
        painter.rect_filled(segment, 1.5, TRACK);
        let full = (share * 4.0 - k as f32).clamp(0.0, 1.0);
        if full > 0.0 {
            let level = egui::Rect::from_min_max(
                pos2(bar.left(), bottom - height * full),
                pos2(bar.right(), bottom),
            );
            let round = if full < 1.0 {
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 2,
                    se: 2,
                }
            } else {
                egui::CornerRadius::same(2)
            };
            painter.rect_filled(level, round, color);
        }
    }
    fuel_pump(painter, symbol, light);
}

/// The coolant's temperature (as the needle shows it) from C to H under a thermometer in waves: the scale's top
/// marked red from where the engine takes damage, the pointer (and the symbol) blue while
/// cold, orange when hot and red past the limit.
fn coolant_gauge(painter: &egui::Painter, column: egui::Rect, kelvin: f32) {
    let (symbol, bar) = gauge_layout(painter, column, ("H", "C"));
    let (cold, hot) = COOLANT_SCALE;
    let y = |kelvin: f32| {
        let share = ((kelvin - cold) / (hot - cold)).clamp(0.0, 1.0);
        bar.bottom() - share * bar.height()
    };
    painter.rect_filled(bar, 1.5, TRACK);
    let danger = egui::Rect::from_min_max(bar.min, pos2(bar.right(), y(COOLANT_LIMIT)));
    let top = egui::CornerRadius {
        nw: 2,
        ne: 2,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(danger, top, RED.gamma_multiply(0.6));
    for quarter in 1..4 {
        let at = bar.bottom() - quarter as f32 * bar.height() / 4.0;
        let length = if quarter == 2 { 4.0 } else { 2.5 };
        painter.line_segment(
            [
                pos2(bar.right() + 1.0, at),
                pos2(bar.right() + 1.0 + length, at),
            ],
            Stroke::new(1.0, DIM),
        );
    }
    let (color, light) = if kelvin >= COOLANT_LIMIT {
        (RED, RED)
    } else if kelvin >= COOLANT_HOT {
        (ORANGE, ORANGE)
    } else if kelvin < COOLANT_COLD {
        (BLUE, BLUE)
    } else {
        (BAR, DIM)
    };
    let at = y(kelvin);
    let pointer = egui::Rect::from_min_max(
        pos2(bar.left() - 2.0, at - 1.5),
        pos2(bar.right() + 2.0, at + 1.5),
    );
    painter.rect_filled(pointer, 1.0, color);
    thermometer(painter, symbol, light);
}

/// The fuel pump symbol, about 14 points square around `at`: the pump with its window,
/// standing on a base, the hose from its side up to the nozzle.
fn fuel_pump(painter: &egui::Painter, at: Pos2, color: Color32) {
    let body = egui::Rect::from_min_max(at + vec2(-6.0, -7.0), at + vec2(1.5, 6.0));
    painter.rect_filled(body, 1.0, color);
    let window = egui::Rect::from_min_max(at + vec2(-4.5, -5.5), at + vec2(0.0, -2.0));
    painter.rect_filled(window, 0.5, BACKGROUND);
    let base = egui::Rect::from_min_max(at + vec2(-7.5, 5.5), at + vec2(3.0, 7.5));
    painter.rect_filled(base, 0.5, color);
    let hose = [
        at + vec2(1.5, 2.5),
        at + vec2(5.0, 2.5),
        at + vec2(5.0, -4.0),
        at + vec2(2.5, -6.5),
    ];
    painter.add(Shape::line(hose.to_vec(), Stroke::new(1.5, color)));
}

/// The coolant temperature symbol, about 14 points square around `at`: a thermometer
/// with two marks dipped in two waves.
fn thermometer(painter: &egui::Painter, at: Pos2, color: Color32) {
    painter.line_segment(
        [at + vec2(0.0, -7.0), at + vec2(0.0, 1.0)],
        Stroke::new(2.5, color),
    );
    painter.circle_filled(at + vec2(0.0, 2.0), 2.6, color);
    for y in [-5.0, -2.5] {
        painter.line_segment(
            [at + vec2(2.0, y), at + vec2(4.5, y)],
            Stroke::new(1.2, color),
        );
    }
    let wave = |y: f32, from: f32, to: f32| {
        let points = (0..=12)
            .map(|i| {
                let x = from + (to - from) * i as f32 / 12.0;
                at + vec2(x, y + (x * 1.4).sin())
            })
            .collect();
        Shape::line(points, Stroke::new(1.2, color))
    };
    painter.add(wave(4.5, -7.5, -3.8));
    painter.add(wave(4.5, 3.8, 7.5));
    painter.add(wave(7.5, -7.5, 7.5));
}

/// Blue for a cold tread, green at the compound's best temperature, red once it's well
/// overheated.
fn warmth_color(compound: &Compound, temperature: f32) -> Color32 {
    let blue = BLUE;
    let green = Color32::from_rgb(90, 210, 110);
    let red = Color32::from_rgb(240, 70, 50);
    if temperature < compound.best {
        let cold = (compound.best - temperature) / (compound.best - AMBIENT);
        green.lerp_to_gamma(blue, cold.clamp(0.0, 1.0))
    } else {
        let hot = (temperature - compound.best) / (2.0 * compound.overheat);
        green.lerp_to_gamma(red, hot.clamp(0.0, 1.0))
    }
}
