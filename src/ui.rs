//! The windows' common ground (egui, keeping its input from the game, the tool windows),
//! and the sandbox's panel (F2): telemetry and live setup tuning for the active car. The
//! panel's wheel settings are public for other apps' windows.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::collapsing_header::CollapsingState;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, egui};

use crate::camera::OrbitInput;
use crate::car::{ActiveCar, Car, DriverAids};
use crate::debug::DebugView;
use crate::input::{Action, Capture, Capturing, Gamepads, WheelConfig, action_value, input_name};
use crate::physics::engine::AMBIENT_PRESSURE;
use crate::physics::fuel;
use crate::physics::suspension::AxleSetup;
use crate::physics::tire::RIBS;
use crate::sim::{CHASSIS_HZ, SUBSTEPS};
use crate::traffic::{MOST_CARS, Traffic};
use crate::wheel_base::{ForceFeedback, WheelBase};

const CORNER_NAMES: [&str; 4] = ["FL", "FR", "RL", "RR"];

/// egui, with its input kept from the game.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_systems(Startup, absorb_input)
            .add_systems(PreUpdate, block_orbit);
    }
}

/// The sandbox's panel (F2, with `DebugPlugin`): the car's state, telemetry, setup
/// sliders, the wheel and pedals, and the controls.
pub struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            EguiPrimaryContextPass,
            panel.run_if(|view: Res<DebugView>| view.ui),
        );
    }
}

/// Typing into a field (or clicking on the panels) shouldn't drive the car: while egui
/// uses the keyboard or pointer, the game doesn't see them.
fn absorb_input(mut settings: ResMut<EguiGlobalSettings>) {
    settings.enable_absorb_bevy_input_system = true;
}

/// How far the stack of collapsed tool windows is from the top right corner, and how far
/// apart their title bars are.
const TOOL_MARGIN: f32 = 10.0;
const TOOL_SPACING: f32 = 30.0;

/// What a tool window needs to know: its key, and whether the car's panel is shown.
#[derive(SystemParam)]
pub struct ToolInput<'w> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    view: Res<'w, DebugView>,
}

/// A tool window (F3 to F6): always there, but collapsed to its title bar until its key
/// or a click on the bar expands it, so it can be found without knowing the key.
/// Collapsed, it hides with the car's panel (F2). The bars stack up in the top right
/// corner (expanded, a window grows to the left and down), `slot` is the place in the
/// stack. `expanded` keeps whether it was expanded last frame. Gives the window to
/// show and whether it was just expanded, or nothing while it's hidden.
pub fn tool_window(
    ctx: &egui::Context,
    (name, key): (&str, KeyCode),
    slot: usize,
    input: &ToolInput,
    expanded: &mut bool,
) -> Option<(egui::Window<'static>, bool)> {
    let id = egui::Id::new(name);
    let mut state = CollapsingState::load_with_default_open(ctx, id.with("collapsing"), false);
    if input.keys.just_pressed(key) {
        state.set_open(!state.is_open());
        state.store(ctx);
    }
    let opened = state.is_open() && !*expanded;
    *expanded = state.is_open();
    if opened {
        // Over the other windows and the bars below it.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
    }
    if !*expanded && !input.view.ui {
        return None;
    }
    let window = egui::Window::new(format!("{name}  ({key:?})"))
        .id(id)
        .default_open(false)
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(
            ctx.content_rect().right_top()
                + egui::vec2(-TOOL_MARGIN, TOOL_MARGIN + slot as f32 * TOOL_SPACING),
        );
    Some((window, opened))
}

/// Collapses the tool window `name` to its title bar.
pub fn collapse_tool(ctx: &egui::Context, name: &str) {
    let id = egui::Id::new(name).with("collapsing");
    let mut state = CollapsingState::load_with_default_open(ctx, id, false);
    state.set_open(false);
    state.store(ctx);
}

/// Mouse input over the panel shouldn't orbit or zoom the camera.
fn block_orbit(wants: Res<EguiWantsInput>, mut orbit: ResMut<OrbitInput>) {
    orbit.0 = !(wants.wants_pointer_input() || wants.is_pointer_over_area());
}

/// Everything the wheel section of the panel shows and changes.
#[derive(SystemParam)]
pub struct WheelPanel<'w, 's> {
    config: ResMut<'w, WheelConfig>,
    capture: ResMut<'w, Capture>,
    gamepads: Gamepads<'w, 's>,
    base: Res<'w, WheelBase>,
    ffb: ResMut<'w, ForceFeedback>,
}

impl WheelPanel<'_, '_> {
    /// The bindings with live values, the wheel's settings and force feedback.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let Self {
            config,
            capture,
            gamepads,
            base,
            ffb,
        } = self;
        wheel(ui, config, capture, gamepads, base);
        force_feedback(ui, config, ffb);
    }
}

/// Where the map and its heights come from.
pub fn credits(ui: &mut egui::Ui) {
    ui.collapsing("Map: Gerach to Rattelsdorf and Baunach, Bavaria", |ui| {
        ui.label("Map data © OpenStreetMap contributors (ODbL)");
        ui.label("Elevation: DGM1 © Bayerische Vermessungsverwaltung (CC BY 4.0)");
    });
}

fn panel(
    mut contexts: EguiContexts,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut wheel_panel: WheelPanel,
    mut traffic: ResMut<Traffic>,
    mut aids: ResMut<DriverAids>,
) -> Result {
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    egui::Window::new(car.setup.name)
        .default_pos([10.0, 10.0])
        .default_width(340.0)
        .vscroll(true)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(format!(
                "{:.0} km/h    {:.0} kg, {:.0} % on the front axle",
                car.forward_speed() * 3.6,
                car.setup.mass_with(car.fuel.litres()),
                car.setup.front_weight_with(car.fuel.litres()) * 100.0
            ));
            let gear = match car.drivetrain.selected_gear() {
                -1 => "R".to_string(),
                0 => "N".to_string(),
                g => g.to_string(),
            };
            ui.horizontal(|ui| {
                ui.label(format!("Gear {gear}    {:.0} rpm", car.drivetrain.rpm));
                aids.show(ui);
            });
            engine(ui, &car);
            fuel(ui, &car);
            ui.add(
                egui::Slider::new(&mut traffic.cars, 0..=MOST_CARS)
                    .text("Traffic density (cars around)"),
            );
            ui.label(format!(
                "Chassis {CHASSIS_HZ:.0} Hz, corners {:.0} Hz ({SUBSTEPS} substeps)",
                CHASSIS_HZ * SUBSTEPS as f64
            ));
            ui.separator();
            telemetry(ui, &car);
            ui.collapsing("Setup", |ui| setup(ui, &mut car));
            ui.collapsing("Wheel and pedals", |ui| wheel_panel.show(ui));
            ui.collapsing("Controls", |ui| {
                ui.label("W/S or arrows: throttle, brake");
                ui.label("Q/E: shift down/up    G: automatic gearbox");
                ui.label("I: start the engine (after stalling with a clutch pedal)");
                ui.label("(automatic: S at a stop selects reverse)");
                ui.label("A/D: steer    Space: handbrake");
                ui.label("R: reset car    F6: showroom (pick a car)");
                ui.label("F1: gizmos    F2: this panel    F3: ECU tune    F4: garage    F5: dyno    M: mute");
                ui.label("C: chase/driver view    K: map");
                ui.label("Wheel and pedals: bind them under \"Wheel and pedals\"");
                ui.label("Left-drag: orbit or look around    Scroll: zoom");
            });
            credits(ui);
        });
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
pub fn engine(ui: &mut egui::Ui, car: &Car) {
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
    if let Some(turbo) = &build.hardware.turbo {
        ui.label(format!(
            "boost {:.2} bar    turbo {:.0}k rpm ({:.0} % of its limit)    wastegate {:.0} %    \
             charge {:.0} °C",
            (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5,
            engine.turbo_speed * 60.0 / std::f32::consts::TAU / 1000.0,
            engine.turbo_speed / turbo.max_speed() * 100.0,
            engine.wastegate * 100.0,
            engine.charge_temperature - 273.15,
        ));
    }
    let thermal = &engine.thermal;
    ui.label(format!(
        "coolant {:.0} °C    oil {:.0} °C{}    knock {:.1}°    wear {:.0} %",
        thermal.coolant - 273.15,
        thermal.oil - 273.15,
        if thermal.fan { "    fan" } else { "" },
        cycle.knock,
        engine.damage.worst(build.hardware.cylinders as usize) * 100.0,
    ));
}

/// What's in the tank, what the engine burns, and how far it goes on what's left.
pub fn fuel(ui: &mut egui::Ui, car: &Car) {
    let (fuel, tank) = (&car.fuel, &car.setup.tank);
    let per_hour = car.drivetrain.engine.fuel_flow / fuel::DENSITY * 3600.0;
    let speed = car.body.velocity.length();
    let now = if speed > 3.0 {
        format!("{:.1} L/100 km", per_hour / (speed * 3.6) * 100.0)
    } else {
        format!("{per_hour:.1} L/h")
    };
    let starving = if fuel.supply < 0.95 {
        "    STARVING"
    } else {
        ""
    };
    ui.label(format!(
        "fuel {:.1} of {:.0} L    now {now}{starving}",
        fuel.litres(),
        tank.litres
    ));
    let average = fuel
        .average()
        .map_or("average and range after 1 km".into(), |average| {
            format!(
                "average {average:.1} L/100 km    range {:.0} km",
                fuel.litres() / average * 100.0
            )
        });
    ui.label(format!(
        "trip (since filling up) {:.1} km, {:.2} L    {average}",
        fuel.distance / 1000.0,
        fuel.burnt,
    ))
    .on_hover_text("Starts again when the tank is filled: R, or the fuel load slider");
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
            "Surface °C, in to out",
            "Core °C",
            "Carcass °C",
            "bar",
            "Wear %",
            "Grip %",
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
            // Against the road while on it (else against the body).
            let camber = match corner.contact {
                Some(contact) => contact.patch.camber * corner.side,
                None => corner.pose.camber,
            };
            ui.label(format!("{:.2}", camber.to_degrees()));
            ui.label(format!("{:.0}", corner.travel * 1000.0));
            ui.label(format!("{:.0}", corner.omega));
            let tire = &car.setup.axle(corner.front).tire;
            let tread = &corner.tread;
            let ribs = |temperatures: [f32; RIBS]| {
                let ribs: Vec<String> = temperatures.iter().map(|t| format!("{t:.0}")).collect();
                ribs.join(" ")
            };
            ui.label(ribs(tread.surface));
            ui.label(ribs(tread.core));
            ui.label(format!("{:.0}", tread.carcass));
            ui.label(format!("{:.2}", tire.pressure(tread)));
            ui.label(format!("{:.1}", tread.wear() * 100.0));
            ui.label(format!("{:.0}", tire.condition(&corner.tread) * 100.0));
            ui.end_row();
        }
    });
}

/// The engine (built in the garage) and the fuel.
fn engine_setup(ui: &mut egui::Ui, car: &mut Car) {
    ui.strong("Engine");
    let engine = &mut car.setup.engine;
    ui.label(format!(
        "{}, {} (F4: garage)",
        engine.build.hardware.name,
        engine.drive.name()
    ));
    let build = &mut engine.build;
    ui.add(egui::Slider::new(&mut build.octane, 89.0..=102.0).text("Fuel octane (RON)"))
        .on_hover_text("The factory tune is for 95; lower knocks");
}

fn setup(ui: &mut egui::Ui, car: &mut Car) {
    engine_setup(ui, car);
    let mut fill = car.fuel.litres() / car.setup.tank.litres * 100.0;
    let tank = car.setup.tank.litres;
    if ui
        .add(
            egui::Slider::new(&mut fill, 0.0..=100.0).text(format!("Fuel load % (of {tank:.0} L)")),
        )
        .on_hover_text("Fills (or drains) the tank to this now and on every reset (R)")
        .changed()
    {
        car.refuel(fill / 100.0);
    }
    let setup = &mut car.setup;
    for (name, axle) in [("Front", &mut setup.front), ("Rear", &mut setup.rear)] {
        ui.strong(name);
        axle_sliders(ui, axle);
    }
    ui.strong("Tires");
    ui.label(setup.front.tire.compound.name)
        .on_hover_text(setup.front.tire.compound.about);
    let mut mu = setup.front.tire.mu;
    ui.add(egui::Slider::new(&mut mu, 0.5..=1.8).text("Peak μ (warm, new)"));
    setup.front.tire.mu = mu;
    setup.rear.tire.mu = mu;
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
