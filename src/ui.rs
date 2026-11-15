//! Telemetry and live setup tuning for the active car.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use crate::camera::OrbitInput;
use crate::car::{ActiveCar, Car};
use crate::debug::DebugView;
use crate::physics::suspension::AxleSetup;
use crate::sim::{CHASSIS_HZ, SUBSTEPS};

const CORNER_NAMES: [&str; 4] = ["FL", "FR", "RL", "RR"];

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_systems(PreUpdate, block_orbit)
            .add_systems(
                EguiPrimaryContextPass,
                panel.run_if(|view: Res<DebugView>| view.ui),
            );
    }
}

/// Mouse input over the panel shouldn't orbit or zoom the camera.
fn block_orbit(wants: Res<EguiWantsInput>, mut orbit: ResMut<OrbitInput>) {
    orbit.0 = !(wants.wants_pointer_input() || wants.is_pointer_over_area());
}

fn panel(mut contexts: EguiContexts, active: Res<ActiveCar>, mut cars: Query<&mut Car>) -> Result {
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    egui::Window::new(car.setup.name)
        .default_pos([10.0, 10.0])
        .default_width(340.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(format!("{:.0} km/h", car.forward_speed() * 3.6));
            ui.label(format!(
                "Chassis {CHASSIS_HZ:.0} Hz, corners {:.0} Hz ({SUBSTEPS} substeps)",
                CHASSIS_HZ * SUBSTEPS as f64
            ));
            ui.separator();
            telemetry(ui, &car);
            ui.collapsing("Setup", |ui| setup(ui, &mut car));
            ui.collapsing("Controls", |ui| {
                ui.label("W/S or arrows: throttle, brake, reverse");
                ui.label("A/D: steer    Space: handbrake");
                ui.label("Tab: next car    R: reset car");
                ui.label("F1: gizmos    F2: this panel    M: mute");
                ui.label("Left-drag: orbit    Scroll: zoom");
            });
            ui.collapsing("Map: Gerach to Rattelsdorf, Bavaria", |ui| {
                ui.label("Map data © OpenStreetMap contributors (ODbL)");
                ui.label("Elevation: DGM1 © Bayerische Vermessungsverwaltung (CC BY 4.0)");
            });
        });
    Ok(())
}

fn telemetry(ui: &mut egui::Ui, car: &Car) {
    egui::Grid::new("telemetry").striped(true).show(ui, |ui| {
        for header in [
            "",
            "Load N",
            "Slip",
            "Slip °",
            "Camber °",
            "Travel mm",
            "ω rad/s",
        ] {
            ui.label(header);
        }
        ui.end_row();
        for (name, corner) in CORNER_NAMES.iter().zip(&car.corners) {
            let contact = corner.contact.unwrap_or_default();
            ui.label(*name);
            ui.label(format!("{:.0}", contact.load));
            ui.label(format!("{:.2}", contact.tire.slip_ratio));
            ui.label(format!("{:.1}", contact.tire.slip_angle.to_degrees()));
            ui.label(format!("{:.2}", corner.pose.camber.to_degrees()));
            ui.label(format!("{:.0}", corner.travel * 1000.0));
            ui.label(format!("{:.0}", corner.omega));
            ui.end_row();
        }
    });
}

fn setup(ui: &mut egui::Ui, car: &mut Car) {
    let setup = &mut car.setup;
    for (name, axle) in [("Front", &mut setup.front), ("Rear", &mut setup.rear)] {
        ui.strong(name);
        axle_sliders(ui, axle);
    }
    ui.strong("Tires");
    let mut tire = setup.front.tire;
    ui.add(egui::Slider::new(&mut tire.mu, 0.5..=1.8).text("Peak μ"));
    ui.add(egui::Slider::new(&mut tire.b_long, 5.0..=25.0).text("B longitudinal"));
    ui.add(egui::Slider::new(&mut tire.b_lat, 5.0..=25.0).text("B lateral"));
    setup.front.tire = tire;
    setup.rear.tire = tire;
    ui.strong("Brakes");
    ui.add(egui::Slider::new(&mut setup.brake_torque, 1_000.0..=6_000.0).text("Torque N·m"));
    ui.add(egui::Slider::new(&mut setup.brake_bias, 0.4..=0.9).text("Front share"));
}

fn axle_sliders(ui: &mut egui::Ui, axle: &mut AxleSetup) {
    ui.add(egui::Slider::new(&mut axle.spring_rate, 20_000.0..=250_000.0).text("Spring N/m"));
    ui.add(egui::Slider::new(&mut axle.damping, 500.0..=15_000.0).text("Damper N·s/m"));
    let mut camber = axle.geometry.static_camber.to_degrees();
    ui.add(egui::Slider::new(&mut camber, -5.0..=1.0).text("Static camber °"));
    axle.geometry.static_camber = camber.to_radians();
    // Moving the upper arm's inner pivot outward shortens the arm: more camber gain.
    ui.add(egui::Slider::new(&mut axle.geometry.upper_pivot.x, 0.25..=0.6).text("Upper pivot x m"));
}
