//! The windows' common ground (egui, keeping its input from the game), and the sandbox's
//! telemetry panel (2). Everything a driver sets is in the central menu (`menu`).

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPreUpdateSet, EguiPrimaryContextPass, egui};

use crate::car::{ActiveCar, Car};
use crate::debug::DebugView;
use crate::physics::engine::AMBIENT_PRESSURE;
use crate::physics::fuel;
use crate::physics::tire::RIBS;
use crate::sim::{CHASSIS_HZ, SUBSTEPS};

const CORNER_NAMES: [&str; 4] = ["FL", "FR", "RL", "RR"];

/// egui, with its input kept from the game.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            EguiPlugin::default(),
            ShieldPlugin { pointer: true },
            crate::menu::LookPlugin,
        ));
    }
}

/// Where `ShieldPlugin` clears the input egui has (in `PreUpdate`).
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shield;

/// Keeps egui's input from the game: while egui uses the keyboard (a text field has focus,
/// a popup is open), no key reaches the game's systems, and with `pointer`, while egui
/// uses the pointer (it's over a window), neither do the mouse's buttons, motion and
/// wheel. So the game's systems read their input as it is, never asking egui. The wheel
/// and pedals aren't held back: they can't type, and a held brake must stay held.
pub struct ShieldPlugin {
    /// Also the mouse (the editor checks the pointer itself: holding the buttons back
    /// would drop a node being dragged as soon as the pointer crosses its panel).
    pub pointer: bool,
}

impl Plugin for ShieldPlugin {
    fn build(&self, app: &mut App) {
        let pointer = self.pointer;
        app.add_systems(
            PreUpdate,
            (move |wants: Res<EguiWantsInput>,
                   mut keys: ResMut<ButtonInput<KeyCode>>,
                   mut buttons: ResMut<ButtonInput<MouseButton>>,
                   mut motion: ResMut<AccumulatedMouseMotion>,
                   mut scroll: ResMut<AccumulatedMouseScroll>| {
                if wants.wants_any_keyboard_input() {
                    keys.reset_all();
                }
                if pointer && wants.wants_any_pointer_input() {
                    buttons.reset_all();
                    motion.delta = Vec2::ZERO;
                    scroll.delta = Vec2::ZERO;
                }
            })
            .in_set(Shield)
            .after(EguiPreUpdateSet::ProcessInput),
        );
    }
}

/// The sandbox's telemetry panel (2, with `DebugPlugin`): the car's state, the engine,
/// the fuel and the tires.
pub struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            EguiPrimaryContextPass,
            panel.run_if(|view: Res<DebugView>| view.ui),
        );
    }
}

fn panel(mut contexts: EguiContexts, active: Res<ActiveCar>, cars: Query<&Car>) -> Result {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    egui::Window::new(&car.setup.name)
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
            ui.label(format!("Gear {gear}    {:.0} rpm", car.drivetrain.rpm));
            drivetrain(ui, car);
            engine(ui, car);
            fuel(ui, car);
            ui.label(format!(
                "Chassis {CHASSIS_HZ:.0} Hz, corners {:.0} Hz ({SUBSTEPS} substeps)",
                CHASSIS_HZ * SUBSTEPS as f64
            ));
            ui.separator();
            telemetry(ui, car);
        });
    Ok(())
}

/// Manifold pressure, spark, torque, exhaust temperature and what the lambda sensor says.
pub fn engine(ui: &mut egui::Ui, car: &Car) {
    let (drivetrain, build) = (&car.drivetrain, &car.setup.engine.build);
    let engine = &drivetrain.engine;
    let cycle = engine.cycle(build, drivetrain.rpm);
    // Without combustion (stalled, or fuel cut), the exhaust is just air.
    let exhaust = if cycle.lambda.is_finite() {
        format!("{:.0} °C", cycle.exhaust_temperature - 273.15)
    } else {
        "-".to_string()
    };
    ui.label(format!(
        "{:.0} kPa    spark {:.0}°    {:.0} N·m    exhaust {exhaust}",
        engine.intake_pressure / 1000.0,
        engine.actuators.spark,
        engine.torque(build, drivetrain.rpm),
    ));
    // What the lambda sensor says: on the road there's no exhaust analyser.
    ui.label(format!("λ sensor: {}", crate::ecu_ui::lambda_sensor(car)));
    if let Some(turbo) = &build.hardware.turbo {
        // Twin turbos turn alike (one shaft state for them all).
        let turbos = match build.hardware.turbos {
            0 | 1 => "turbo".to_string(),
            2 => "twin turbos".to_string(),
            n => format!("{n} turbos"),
        };
        ui.label(format!(
            "boost {:.2} bar    {turbos} {:.0}k rpm ({:.0} % of the limit)    wastegate {:.0} %    \
             charge {:.0} °C",
            (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5,
            engine.turbo_speed * 60.0 / std::f32::consts::TAU / 1000.0,
            engine.turbo_speed / turbo.max_speed() * 100.0,
            engine.wastegate * 100.0,
            engine.charge_temperature - 273.15,
        ));
    }
    if build.hardware.supercharger.is_some() {
        ui.label(format!(
            "boost {:.2} bar    charge {:.0} °C",
            (engine.boost_pressure - AMBIENT_PRESSURE) / 1e5,
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

/// How worn the clutch and the gears are, and the air's downforce now.
fn drivetrain(ui: &mut egui::Ui, car: &Car) {
    let drivetrain = &car.drivetrain;
    let gears = car.setup.engine.gears.len().min(drivetrain.gear_wear.len());
    let gears: Vec<String> = (drivetrain.gear_wear[..gears].iter())
        .map(|wear| format!("{:.0}", wear * 100.0))
        .collect();
    let lowered = [0, 2].map(|i| (car.corners[i].travel + car.corners[i + 1].travel) / 2.0);
    let pressure = 0.6 * car.body.velocity.length_squared();
    let [front, rear] = car.setup.downforce(lowered).map(|area| area * pressure);
    ui.label(format!(
        "clutch wear {:.1} %    gears' wear {} %    downforce {front:.0} / {rear:.0} N",
        drivetrain.clutch_wear * 100.0,
        gears.join(" "),
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
        "fuel {:.1} of {:.0} L, {:.1} octane    now {now}{starving}",
        fuel.litres(),
        tank.litres,
        fuel.octane()
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
    .on_hover_text("Starts again when the tank is filled at a gas station");
}

fn telemetry(ui: &mut egui::Ui, car: &Car) {
    egui::Grid::new("telemetry").striped(true).show(ui, |ui| {
        for header in [
            "",
            "Load N",
            "Ground",
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
            "Brake °C",
            "Pads %",
        ] {
            ui.label(header);
        }
        ui.end_row();
        for (name, corner) in CORNER_NAMES.iter().zip(&car.corners) {
            let contact = corner.contact.unwrap_or_default();
            ui.label(*name);
            ui.label(format!("{:.0}", contact.load));
            ui.label(corner.contact.map_or("–", |c| c.surface.name()));
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
            ui.label(format!("{:.0}", corner.brake.temperature - 273.15));
            ui.label(format!("{:.1}", corner.brake.wear * 100.0));
            ui.end_row();
        }
    });
}
