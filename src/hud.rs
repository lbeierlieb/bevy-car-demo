//! Driving HUD in the bottom right corner: speed and gear in the middle, with a rev counter
//! arc around them, and the tires' temperatures and wear and the fuel left of it.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Shape, Stroke, pos2, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};
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
        app.add_systems(EguiPrimaryContextPass, hud);
    }
}

fn hud(mut contexts: EguiContexts, active: Res<ActiveCar>, cars: Query<&Car>) -> Result {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
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

            // Speed above gear in the middle.
            let speed = (car.forward_speed().abs() * 3.6).round();
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
            let cylinders = engine.build.hardware.cylinders as usize;
            let failure = car.drivetrain.engine.damage.failures(cylinders).pop();
            if !car.drivetrain.running {
                let blown = car.drivetrain.engine.blown(&engine.build.hardware);
                painter.text(
                    center + vec2(0.0, RADIUS + 4.0),
                    Align2::CENTER_CENTER,
                    if blown {
                        "ENGINE BLOWN (R)"
                    } else {
                        "STALLED (I)"
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
            } else if car.drivetrain.automatic {
                painter.text(
                    center + vec2(0.0, RADIUS + 4.0),
                    Align2::CENTER_CENTER,
                    "AUTO",
                    FontId::proportional(10.0),
                    DIM,
                );
            }
        });

    tires(contexts.ctx_mut()?, car);
    Ok(())
}

/// Tire size in the tire display, and the gap between them.
const TIRE: egui::Vec2 = vec2(30.0, 34.0);
const TIRE_GAP: egui::Vec2 = vec2(14.0, 22.0);
/// Room under each tire for its pressure.
const PRESSURE_ROW: f32 = 14.0;
/// Width of the fuel gauge right of the tires, and its gap to them.
const FUEL_WIDTH: f32 = 8.0;
const FUEL_GAP: f32 = 12.0;
/// Below this share of the tank the fuel gauge turns orange.
const FUEL_LOW: f32 = 0.15;

/// The four tires left of the gauge, seen from above: each rib (inside towards the car's
/// middle) coloured by how warm its surface is against the compound's window (blue cold,
/// green right, red overheated) with its worn share darkened from the top, the core
/// temperature in the middle, and the pressure below. Right of them, the fuel gauge:
/// orange when low, red while the engine starves.
fn tires(ctx: &egui::Context, car: &Car) {
    let size = 2.0 * TIRE + TIRE_GAP + vec2(16.0 + FUEL_GAP + FUEL_WIDTH, 16.0 + PRESSURE_ROW);
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

            let gauge = egui::Rect::from_min_max(
                rect.right_top() + vec2(-8.0 - FUEL_WIDTH, 8.0),
                rect.right_bottom() + vec2(-8.0, -8.0),
            );
            painter.rect_filled(gauge, 3.0, TRACK);
            let share = (car.fuel.litres() / car.setup.tank.litres).clamp(0.0, 1.0);
            let level = egui::Rect::from_min_max(
                pos2(gauge.min.x, gauge.max.y - gauge.height() * share),
                gauge.max,
            );
            let color = if car.fuel.supply < 0.95 {
                RED
            } else if share < FUEL_LOW {
                ORANGE
            } else {
                BAR
            };
            painter.rect_filled(level, 3.0, color);
        });
}

/// Blue for a cold tread, green at the compound's best temperature, red once it's well
/// overheated.
fn warmth_color(compound: &Compound, temperature: f32) -> Color32 {
    let blue = Color32::from_rgb(90, 150, 240);
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
