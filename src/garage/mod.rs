//! The car dealers' showroom, the workshops of car workshops, tuning shops and tire shops,
//! and the garage park (see `shops`). The showroom has every trim as it leaves the factory,
//! in its hall (`showroom`), the car picked on a turntable; the workshops fit parts
//! (`fit`), the car up on a lift (`bay`). The alignment (`alignment`) is set at alignment
//! shops.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::car::{ActiveCar, Car, CarSetup};
use crate::catalogue::Shared;
use crate::chassis::{BRAKE_BALANCES, BRAKE_FORCES, CarBuild, PRESSURES, TOES};
use crate::items::{Kind, Panel};
use crate::menu::kit;

mod bay;
pub mod fit;
pub mod park;
mod showroom;

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((bay::BayPlugin, showroom::ShowroomPlugin, park::ParkPlugin))
            .add_systems(Update, open);
    }
}

#[derive(Resource)]
struct Garage {
    /// The showroom: the chassis looked at, and the engine option picked for it.
    showroom: Showroom,
    /// The catalogue it's for (another server's: afresh).
    catalogue: u64,
    status: String,
}

#[derive(Clone, Copy)]
struct Showroom {
    chassis: usize,
    /// The trim shown, by its place among the chassis' (`items::trims_of`).
    trim: usize,
}

impl Showroom {
    /// The trim shown.
    fn trim<'a>(
        &self,
        catalogue: &'a crate::catalogue::Catalogue,
    ) -> Option<&'a crate::items::Trim> {
        crate::items::trims_of(catalogue, self.chassis)
            .get(self.trim)
            .copied()
    }
}

impl Garage {
    /// The shops, for `car`'s catalogue.
    fn new(car: &Car) -> Self {
        let setup = &car.setup;
        Self {
            showroom: Showroom {
                chassis: setup.build.chassis,
                trim: 0,
            },
            catalogue: setup.build.catalogue.fingerprint(),
            status: String::new(),
        }
    }
}

/// The shops open once there's a car (the server's first, in its catalogue): again when
/// the catalogue changes (another server).
fn open(
    mut commands: Commands,
    garage: Option<Res<Garage>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    if garage.is_none_or(|garage| garage.catalogue != car.setup.build.catalogue.fingerprint()) {
        commands.insert_resource(Garage::new(car));
    }
}

/// Each tire's load with the car standing (N), front and rear.
pub fn static_loads(setup: &CarSetup) -> [f32; 2] {
    [true, false].map(|front| (setup.corner_mass(front) + setup.axle(front).unsprung_mass) * 9.81)
}

/// Camber (within what the suspension kit allows), cold tire pressure and toe per axle,
/// with the pressure the tires suit standing; and the brakes' force and balance. `setup`
/// is the car as it is now (for the loads and the factory brake balance).
pub fn alignment(ui: &mut egui::Ui, build: &mut CarBuild, setup: &CarSetup) {
    let loads = static_loads(setup);
    for axle in 0..2 {
        let name = ["Front", "Rear"][axle];
        kit::section(ui, name);
        let range = build.camber_range(axle);
        kit::slider(ui, "Camber", &mut build.camber[axle], range, 0.1, |v| {
            format!("{v:.1}°")
        });
        kit::slider(ui, "Toe-in", &mut build.toe[axle], TOES, 0.05, |v| {
            format!("{v:.2}°")
        });
        if let Some(range) = knob_range(setup, Kind::Spring, 2 * axle) {
            kit::slider(
                ui,
                "Ride height",
                &mut build.ride_height[axle],
                range,
                5.0,
                |v| {
                    if v.abs() < 0.5 {
                        "design height".to_string()
                    } else {
                        format!("{v:+.0} mm")
                    }
                },
            );
        }
        let wheel = &build.wheels[axle];
        let suited = wheel
            .tire
            .params(build.rubber(axle))
            .suited_pressure(loads[axle]);
        kit::slider(
            ui,
            "Tire pressure (cold)",
            &mut build.pressure[axle],
            PRESSURES,
            0.05,
            |v| format!("{v:.2} bar, suits {suited:.1}"),
        );
    }
    kit::note(
        ui,
        "Negative camber leans the tops of the wheels in: it evens out the load across the \
         outside tire in a corner, but wears and heats the inside shoulders on the straight. \
         Tires warm up and gain pressure as they work: too little and they carry on their \
         shoulders, too much and on their middle. Toe-in steadies an axle, toe-out makes it \
         keener to turn.",
    );
    if let Some(range) = knob_range(setup, Kind::Body(Panel::RearSpoiler), 0) {
        kit::section(ui, "Rear wing");
        kit::slider(ui, "Angle", &mut build.wing, range, 1.0, |v| {
            format!("{v:.0}°")
        });
        kit::note(
            ui,
            "A steeper wing pushes the rear down harder at speed, and holds the car back \
             more on the straights.",
        );
    }
    kit::section(ui, "Brakes");
    let factory_torque = setup.brake_torque / setup.build.brake_force;
    let factory_bias = setup.brake_bias - setup.build.brake_balance;
    let mut percent = build.brake_force * 100.0;
    kit::slider(
        ui,
        "Force",
        &mut percent,
        BRAKE_FORCES.start() * 100.0..=BRAKE_FORCES.end() * 100.0,
        1.0,
        |v| format!("{v:.0} % ({:.0} N·m)", factory_torque * v / 100.0),
    );
    build.brake_force = percent / 100.0;
    let mut front = (factory_bias + build.brake_balance) * 100.0;
    kit::slider(
        ui,
        "Balance",
        &mut front,
        (factory_bias + BRAKE_BALANCES.start()) * 100.0
            ..=(factory_bias + BRAKE_BALANCES.end()) * 100.0,
        1.0,
        |v| format!("{v:.0} % front (factory {:.0} %)", factory_bias * 100.0),
    );
    build.brake_balance = front / 100.0 - factory_bias;
    kit::note(
        ui,
        "Stronger brakes stop shorter until the tires lock (ABS holds them back). More at \
         the front keeps the rear wheels from locking first; more at the rear stops harder \
         until the rear steps out.",
    );
    build.clamp_alignment();
}

/// The range of the first knob of the `n`th part in a slot of `kind` of the car, if it has
/// such a part and the part a knob (a spring's ride height, a wing's angle).
fn knob_range(setup: &CarSetup, kind: Kind, n: usize) -> Option<std::ops::RangeInclusive<f32>> {
    let catalogue = &setup.build.catalogue;
    let item = setup.car.fitted(catalogue, kind).nth(n)?;
    let knob = item.def(catalogue).knobs.first()?;
    (knob.low < knob.high).then_some(knob.low..=knob.high)
}

/// A chassis' page in the showroom: what it is, its engines and paint, and the button
/// to drive it (true when clicked).
fn brochure(ui: &mut egui::Ui, catalogue: &Shared, shown: &Showroom) {
    let chassis = &catalogue.chassis[shown.chassis];
    let body = &chassis.body;
    kit::section(ui, &chassis.class);
    kit::note(ui, &chassis.about);
    // The trim: its wheels, brakes and drivetrain as well as its engine.
    let Some(setup) =
        (shown.trim(catalogue)).and_then(|trim| CarSetup::of_trim(catalogue, &trim.id))
    else {
        return;
    };
    let (build, parts) = (&setup.build, &setup.engine.parts);
    kit::note(ui, &parts.family().about);
    let gearbox = build.gearbox();
    let mass = setup.mass_with(0.0);
    kit::info(ui, "Engine", &format!("at the {}", build.bay().place));
    kit::info(ui, "Drive", setup.engine.drive.name());
    kit::info(ui, "Gearbox", &gearbox.name.to_lowercase());
    kit::info(ui, "Weight", &format!("{mass:.0} kg"));
    kit::info(
        ui,
        "Size",
        &format!(
            "{:.2} × {:.2} × {:.2} m",
            body.length,
            body.width,
            body.height()
        ),
    );
    kit::info(ui, "Wheelbase", &format!("{:.2} m", chassis.wheelbase));
    let [front, rear] = build.wheels.map(|wheel| catalogue.wheel_label(&wheel));
    if front == rear {
        kit::info(ui, "Tires", &front);
    } else {
        kit::info(ui, "Tires", &format!("{front} / {rear}"));
    }
    kit::info(ui, "Paint", &catalogue.paints[chassis.paint].name);
    kit::note(
        ui,
        "Car workshops swap engines and fit parts; any paint shop changes the paint.",
    );
}
