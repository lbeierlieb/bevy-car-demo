//! Driving HUD in the bottom right corner: speed and gear in the middle, with a rev counter
//! arc around them.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Shape, Stroke, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};

/// Gauge radius (to the middle of the rev arc) and the arc's sweep: 270°, open at the
/// bottom, from 0 rpm at the lower left clockwise to the top of the scale at the lower right.
const RADIUS: f32 = 78.0;
const START: f32 = 0.75 * PI;
const SWEEP: f32 = 1.5 * PI;

const BACKGROUND: Color32 = Color32::from_rgba_premultiplied(10, 12, 16, 190);
const TRACK: Color32 = Color32::from_rgba_premultiplied(70, 74, 82, 255);
const BAR: Color32 = Color32::from_rgb(240, 240, 235);
const RED: Color32 = Color32::from_rgb(230, 50, 40);
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
            painter.add(arc(engine.redline_rpm, top, RADIUS + 7.0, 3.0, RED));
            let bar = if rpm >= engine.redline_rpm { RED } else { BAR };
            painter.add(arc(0.0, rpm.min(top), RADIUS, 10.0, bar));
            // A tick and a number every thousand rpm.
            for k in 0..=(top / 1000.0) as usize {
                let r = k as f32 * 1000.0;
                let color = if r >= engine.redline_rpm { RED } else { DIM };
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
            if !car.drivetrain.running {
                painter.text(
                    center + vec2(0.0, RADIUS + 4.0),
                    Align2::CENTER_CENTER,
                    "STALLED (I)",
                    FontId::proportional(10.0),
                    RED,
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
    Ok(())
}
