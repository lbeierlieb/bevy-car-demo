//! Telemetry and live setup tuning for the active car.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use crate::camera::OrbitInput;
use crate::car::{ActiveCar, Car};
use crate::debug::DebugView;
use crate::input::{Action, Capture, Capturing, Gamepads, WheelConfig, action_value, input_name};
use crate::physics::suspension::AxleSetup;
use crate::sim::{CHASSIS_HZ, SUBSTEPS};
use crate::wheel_base::{ForceFeedback, WheelBase};

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

/// Everything the wheel section of the panel shows and changes.
#[derive(SystemParam)]
struct WheelPanel<'w, 's> {
    config: ResMut<'w, WheelConfig>,
    capture: ResMut<'w, Capture>,
    gamepads: Gamepads<'w, 's>,
    base: Res<'w, WheelBase>,
    ffb: ResMut<'w, ForceFeedback>,
}

fn panel(
    mut contexts: EguiContexts,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut wheel_panel: WheelPanel,
) -> Result {
    let mut automatic = None;
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    egui::Window::new(car.setup.name)
        .default_pos([10.0, 10.0])
        .default_width(340.0)
        .vscroll(true)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(format!("{:.0} km/h", car.forward_speed() * 3.6));
            let gear = match car.drivetrain.selected_gear() {
                -1 => "R".to_string(),
                0 => "N".to_string(),
                g => g.to_string(),
            };
            ui.horizontal(|ui| {
                ui.label(format!("Gear {gear}    {:.0} rpm", car.drivetrain.rpm));
                let mut on = car.drivetrain.automatic;
                if ui.checkbox(&mut on, "Automatic (G)").changed() {
                    automatic = Some(on);
                }
            });
            engine(ui, &car);
            ui.label(format!(
                "Chassis {CHASSIS_HZ:.0} Hz, corners {:.0} Hz ({SUBSTEPS} substeps)",
                CHASSIS_HZ * SUBSTEPS as f64
            ));
            ui.separator();
            telemetry(ui, &car);
            ui.collapsing("Setup", |ui| setup(ui, &mut car));
            ui.collapsing("Wheel and pedals", |ui| {
                let WheelPanel {
                    config,
                    capture,
                    gamepads,
                    base,
                    ffb,
                } = &mut wheel_panel;
                wheel(ui, config, capture, gamepads, base);
                force_feedback(ui, config, ffb);
            });
            ui.collapsing("Controls", |ui| {
                ui.label("W/S or arrows: throttle, brake");
                ui.label("Q/E: shift down/up    G: automatic gearbox");
                ui.label("I: start the engine (after stalling with a clutch pedal)");
                ui.label("(automatic: S at a stop selects reverse)");
                ui.label("A/D: steer    Space: handbrake");
                ui.label("Tab: next car    R: reset car");
                ui.label("F1: gizmos    F2: this panel    M: mute");
                ui.label("C: chase/driver view    K: map");
                ui.label("Wheel and pedals: bind them under \"Wheel and pedals\"");
                ui.label("Left-drag: orbit or look around    Scroll: zoom");
            });
            ui.collapsing("Map: Gerach to Rattelsdorf and Baunach, Bavaria", |ui| {
                ui.label("Map data © OpenStreetMap contributors (ODbL)");
                ui.label("Elevation: DGM1 © Bayerische Vermessungsverwaltung (CC BY 4.0)");
            });
        });
    // The gearbox mode applies to every car, so switching cars keeps it.
    if let Some(on) = automatic {
        for mut car in &mut cars {
            car.drivetrain.automatic = on;
        }
    }
    Ok(())
}

/// Bindings with live values, and the wheel settings.
fn wheel(
    ui: &mut egui::Ui,
    config: &mut WheelConfig,
    capture: &mut Capture,
    gamepads: &Gamepads,
    base: &WheelBase,
) {
    ui.checkbox(&mut config.enabled, "Use wheel and pedals")
        .on_hover_text("Untick to drive with the keyboard only; the bindings are kept");
    let names = gamepads.names();
    if names.is_empty() {
        ui.label("No wheel or gamepad connected.");
    } else {
        ui.label(format!("Connected: {}", names.join(", ")));
    }
    let listening = capture.0.as_ref().map(|c| c.action);
    egui::Grid::new("bindings").striped(true).show(ui, |ui| {
        for action in Action::ALL {
            ui.label(action.label());
            let value = action_value(config, gamepads, action);
            if listening == Some(action) {
                ui.label(match action {
                    Action::Steer => "Turn the wheel left…",
                    Action::Gate(_) => "Move the lever into the gate…",
                    _ if action.analog() => "Press fully, then release…",
                    _ => "Press the button…",
                });
                if ui.button("Cancel").clicked() {
                    capture.0 = None;
                }
            } else {
                match config.binding(action) {
                    Some(binding) => {
                        let bar = match value {
                            Some(v) if action == Action::Steer => (v + 1.0) / 2.0,
                            Some(v) => v,
                            None => 0.0,
                        };
                        ui.add(
                            egui::ProgressBar::new(bar)
                                .desired_width(110.0)
                                .text(input_name(binding.input)),
                        )
                        .on_hover_text(&binding.device);
                    }
                    None => {
                        ui.label("—");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Bind").clicked() {
                        capture.0 = Some(Capturing::new(action));
                    }
                    if config.binding(action).is_some() && ui.button("Clear").clicked() {
                        config.unbind(action);
                    }
                });
            }
            ui.end_row();
        }
    });
    ui.add(
        egui::Slider::new(&mut config.range, 180.0..=1080.0)
            .step_by(10.0)
            .text("Rotation range °"),
    );
    match (base.applied, base.actual) {
        (Some(range), _) => {
            ui.label(format!("Wheel base set to {range:.0}° (restored on exit)"));
        }
        (None, Some(actual)) => {
            ui.label(format!(
                "Wheel base is at {actual:.0}° and can't be changed: soft lock at the range"
            ));
        }
        (None, None) => {
            let mut other = config.base_range.is_some();
            ui.checkbox(&mut other, "Wheel base set to a different range (soft lock)")
                .on_hover_text(
                    "Otherwise set the base to the rotation range (hid-fanatecff on Linux does it itself)",
                );
            config.base_range = other.then(|| {
                let mut range = config.base_range.unwrap_or(900.0);
                ui.add(
                    egui::Slider::new(&mut range, 180.0..=2880.0)
                        .step_by(10.0)
                        .text("Wheel base range °"),
                );
                range
            });
        }
    }
    ui.add(egui::Slider::new(&mut config.deadzone, 0.0..=0.15).text("Pedal dead zone"));
}

fn force_feedback(ui: &mut egui::Ui, config: &mut WheelConfig, ffb: &mut ForceFeedback) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Force feedback");
        ui.checkbox(&mut config.ffb_enabled, "On");
    });
    let Some(device) = &ffb.device else {
        ui.label(match &ffb.problem {
            Some(problem) => format!("No force feedback: {problem}"),
            None => "No force feedback device: bind the steering to a wheel.".into(),
        });
        return;
    };
    ui.label(format!("On {device}"));
    ui.add(egui::Slider::new(&mut config.ffb_gain, 0.0..=1.0).text("Strength"));
    let side = if ffb.level >= 0.0 { "left" } else { "right" };
    let clipping = ffb.level.abs() > 0.99;
    ui.add(
        egui::ProgressBar::new(ffb.level.abs())
            .desired_width(200.0)
            .fill(if clipping {
                egui::Color32::from_rgb(200, 50, 40)
            } else {
                egui::Color32::from_rgb(60, 110, 170)
            })
            .text(format!("{:.0} % {side}", ffb.level.abs() * 100.0)),
    )
    .on_hover_text("Red when the force is clipped at the base's maximum: lower the strength");
    ui.horizontal(|ui| {
        if ui.button("Test: push left").clicked() {
            ffb.test_push();
        }
        ui.checkbox(&mut config.ffb_invert, "Reverse direction")
            .on_hover_text("Tick if the test pushes the wheel to the right");
    });
}

/// Manifold pressure, mixture, spark, torque and exhaust temperature.
fn engine(ui: &mut egui::Ui, car: &Car) {
    let (drivetrain, build) = (&car.drivetrain, &car.setup.engine.build);
    let engine = &drivetrain.engine;
    let cycle = engine.cycle(build, drivetrain.rpm);
    // Without combustion (stalled, or fuel cut), the exhaust is just air.
    let (lambda, exhaust) = if cycle.lambda.is_finite() {
        let celsius = cycle.exhaust_temperature - 273.15;
        (format!("{:.2}", cycle.lambda), format!("{celsius:.0} °C"))
    } else {
        ("-".to_string(), "-".to_string())
    };
    ui.label(format!(
        "{:.0} kPa    λ {lambda}    spark {:.0}°    {:.0} N·m    exhaust {exhaust}",
        engine.intake_pressure / 1000.0,
        engine.actuators.spark,
        engine.torque(build, drivetrain.rpm),
    ));
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
    let mut toe = axle.geometry.static_toe.to_degrees();
    ui.add(egui::Slider::new(&mut toe, -2.0..=2.0).text("Toe-in °"));
    axle.geometry.static_toe = toe.to_radians();
    // Moving the upper arm's inner pivot outward shortens the arm: more camber gain.
    ui.add(egui::Slider::new(&mut axle.geometry.upper_pivot.x, 0.25..=0.6).text("Upper pivot x m"));
}
