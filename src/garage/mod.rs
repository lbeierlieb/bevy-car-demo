//! The car dealers' showroom and the parts shop (see `shops`). The showroom has every car
//! as it leaves the factory; the parts shop builds the active car from parts: what's
//! fitted to the chassis (gearbox, clutch, final drive, differentials, suspension, rims and
//! tires on each axle), which engine (any that fits the engine bay), and the engine's own
//! parts; it repairs the engine and fits new tires. The alignment (`alignment`) is set at
//! alignment shops.
//!
//! The parts shop's window is in its workshop (`bay`), with the car up on a lift; the
//! showroom's in its hall (`showroom`), the car picked on a turntable.
//!
//! Parts on the cylinder side need the engine's maps baked again (see `workshop`), which
//! takes a few seconds, while the car keeps its old engine.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::car::{ActiveCar, Car, CarSetup, heading};
use crate::catalogue::{Catalogue, Shared};
use crate::chassis::{BRAKE_BALANCES, BRAKE_FORCES, CarBuild, PRESSURES, TOES};
use crate::menu::kit;
use crate::net::client::Connection;
use crate::physics::engine::EngineBuild;
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, EngineMaps};
use crate::physics::engine::parts::{Parts, Readout};
use crate::physics::engine::store;
use crate::physics::tire::Tread;
use crate::terrain::Terrain;
use crate::wheels::{self, TireSize, Wheel};
use crate::workshop::{self, Work};

mod bay;
mod showroom;

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((bay::BayPlugin, showroom::ShowroomPlugin))
            .add_systems(Update, (open, bake.run_if(resource_exists::<Garage>)));
    }
}

#[derive(Resource)]
struct Garage {
    /// The showroom: the chassis looked at, and the engine option picked for it.
    showroom: Showroom,
    /// The car the draft was taken from, and what's picked so far: the car's build and
    /// its engine's parts.
    car: Option<usize>,
    draft: (CarBuild, Parts),
    /// The same rims and tires front and rear.
    same_wheels: bool,
    /// Give the new engine a factory tune, rather than keep the ECU's tune.
    retune: bool,
    job: Option<Job>,
    status: String,
}

#[derive(Clone, Copy)]
struct Showroom {
    chassis: usize,
    engine: usize,
}

impl Garage {
    /// The shops, the draft taken from `car`.
    fn new(car: &Car) -> Self {
        let setup = &car.setup;
        Self {
            showroom: Showroom {
                chassis: setup.build.chassis,
                engine: 0,
            },
            car: None,
            same_wheels: setup.build.wheels[0] == setup.build.wheels[1],
            draft: (setup.build.clone(), setup.engine.parts.clone()),
            retune: true,
            job: None,
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
    if garage.is_none_or(|garage| garage.draft.0.catalogue != car.setup.build.catalogue) {
        commands.insert_resource(Garage::new(car));
    }
}

/// An engine being built: for which car, from what, and its maps baking.
struct Job {
    car: usize,
    build: CarBuild,
    parts: Parts,
    hardware: EngineHardware,
    retune: bool,
    /// Baking the maps, once the server said it hasn't got them (or there's none).
    work: Option<Work>,
}

/// The active car and its entity.
fn active<'a>(
    cars: &'a mut Query<(Entity, &mut Car)>,
    index: usize,
) -> Option<(Entity, Mut<'a, Car>)> {
    cars.iter_mut().find(|(_, car)| car.index == index)
}

/// Moves the baking job on, and builds its car when the maps are done.
fn bake(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut garage: ResMut<Garage>,
    mut connection: Option<ResMut<Connection>>,
    mut cars: Query<(Entity, &mut Car)>,
) {
    let Some(job) = &mut garage.job else {
        return;
    };
    let maps = match &mut job.work {
        Some(work) => {
            let Some(baked) = work.poll() else {
                return;
            };
            let Some(maps) = baked else {
                garage.job = None;
                garage.status = "Baking the maps failed.".to_string();
                return;
            };
            workshop::keep(&job.hardware, &maps);
            maps
        }
        // The server's, if it has them (a factory engine's); else baked here.
        None => {
            let print = maps::fingerprint(&job.hardware);
            if let Some(maps) = workshop::at_hand(&job.hardware) {
                maps
            } else {
                match connection.as_deref_mut() {
                    Some(connection) if !connection.lacks_maps(print) => connection.ask_maps(print),
                    _ => job.work = Some(Work::start(&job.hardware)),
                }
                return;
            }
        }
    };
    let job = garage.job.take().expect("a job");
    garage.status = match active(&mut cars, job.car) {
        Some((entity, mut car)) => {
            let made = (job.build, job.parts, job.hardware, maps);
            fit(&mut commands, &terrain, entity, &mut car, made, job.retune);
            "Built.".to_string()
        }
        None => "The car is gone.".to_string(),
    };
}

/// Rebuilds `car` as `build` with the engine built from `parts`. An engine of other
/// parts is new (no wear, at rest), with a factory tune or the tune it had (moved over to
/// a different ECU, which takes the factory's cam control if the old one had none). A
/// car that only changed under the skin carries on as it moves; one with another chassis,
/// paint or wheels is put down afresh where the old one was, in neutral with the parking
/// brake on.
fn fit(
    commands: &mut Commands,
    terrain: &Terrain,
    entity: Entity,
    car: &mut Car,
    (build, parts, hardware, maps): (CarBuild, Parts, EngineHardware, EngineMaps),
    retune: bool,
) {
    let old = &car.setup.engine.build;
    let factory = || Ecu::calibrated(&hardware, &maps, 95.0, parts.ecu);
    let same_family = parts.family == car.setup.engine.parts.family;
    let ecu = if parts == car.setup.engine.parts {
        old.ecu.clone()
    } else if retune || !same_family {
        factory()
    } else {
        let mut ecu = old.ecu.clone();
        if parts.ecu.cam_control() && !ecu.model.cam_control() {
            ecu.cams = factory().cams;
        }
        ecu.model = parts.ecu;
        ecu
    };
    let engine = EngineBuild {
        hardware,
        maps,
        ecu,
        octane: old.octane,
    };
    let same_look = build == car.setup.build;
    let setup = CarSetup::new(build, parts, engine);
    if same_look {
        *car = car.rebuilt(setup);
        return;
    }
    // The body and wheels are the old entity's children: a new car takes its place.
    let at = car.body.position;
    let mut new = Car::spawn_at(
        car.index,
        setup,
        terrain,
        at.x,
        at.z,
        heading(car.body.rotation),
    );
    new.keep_settings(car);
    // At rest, in neutral with the parking brake on, as `Car::place` leaves a car.
    new.drivetrain.neutral();
    new.parking_brake = true;
    commands.entity(entity).despawn();
    commands.spawn((Transform::from_translation(new.body.position), new));
}

/// Takes the parts from the car again (when it comes onto the lift).
fn take_draft(garage: &mut Garage, car: &Car) {
    garage.car = Some(car.index);
    garage.draft = (car.setup.build.clone(), car.setup.engine.parts.clone());
    garage.same_wheels = car.setup.build.wheels[0] == car.setup.build.wheels[1];
    garage.status.clear();
}

/// The parts shop's window (in the workshop, `bay`): building what's picked, what's
/// fitted to the car and its engine, its condition and repairs. True when "Build" was
/// chosen.
fn parts_shop(ui: &mut egui::Ui, garage: &mut Garage, car: &mut Car) -> bool {
    if garage.car != Some(car.index) {
        take_draft(garage, car);
    }
    let build_now = build(ui, garage, car);
    car_parts(ui, &mut garage.draft.0);
    wheel_parts(ui, &mut garage.draft.0, &mut garage.same_wheels);
    engine(ui, garage);
    parts(ui, garage);
    condition(ui, car);
    build_now
}

/// What's fitted to the chassis.
fn car_parts(ui: &mut egui::Ui, draft: &mut CarBuild) {
    let catalogue = draft.catalogue.clone();
    let chassis = &catalogue.chassis[draft.chassis];
    kit::section(
        ui,
        &format!("{} ({})", chassis.name, chassis.class.to_lowercase()),
    );
    let gearboxes: Vec<usize> = catalogue.gearboxes_for(chassis).collect();
    kit::pick(ui, "Gearbox", &mut draft.gearbox, &gearboxes, |g| {
        catalogue.gearboxes[g].name.clone()
    });
    let gearbox = &catalogue.gearboxes[draft.gearbox];
    let clutches: Vec<usize> = (0..catalogue.clutches.len()).collect();
    kit::pick(ui, "Clutch", &mut draft.clutch, &clutches, |clutch| {
        let clutch = &catalogue.clutches[clutch];
        let torque = gearbox.clutch_torque * clutch.torque;
        format!("{} ({torque:.0} N·m)", clutch.name)
    });
    kit::slider(
        ui,
        "Final drive",
        &mut draft.final_drive,
        2.5..=5.5,
        0.05,
        |v| format!("{v:.2}:1"),
    );
    kit::toggle(ui, "Limited-slip differentials", &mut draft.limited_slip);
    let mut suspension = draft.suspension;
    let kits: Vec<usize> = (0..catalogue.suspensions.len()).collect();
    if kit::pick(ui, "Suspension", &mut suspension, &kits, |k| {
        catalogue.suspensions[k].name.clone()
    }) {
        draft.set_suspension(suspension);
    }
    for (axle, place) in ["Front anti-roll bar", "Rear anti-roll bar"]
        .into_iter()
        .enumerate()
    {
        let bars = &chassis.bars[axle];
        let sizes: Vec<usize> = (0..bars.sizes.len()).collect();
        kit::pick(ui, place, &mut draft.bars[axle], &sizes, |bar| {
            let label = bars.label(bar);
            if bar == bars.factory {
                format!("{label}, factory")
            } else {
                label
            }
        });
    }
    kit::note(
        ui,
        "An engine with more torque than the clutch passes makes it slip. A shorter final \
         drive (a higher number) pulls harder and tops out sooner. Open differentials let a \
         wheel without grip spin the torque away. Stiffer suspension kits come aligned to \
         more negative camber. A stiffer bar at the front makes the car push wider, at the \
         rear turn in more and step out sooner. Another car: at a car dealer.",
    );
}

/// The rims and tires on each axle, or the same all round.
fn wheel_parts(ui: &mut egui::Ui, draft: &mut CarBuild, same: &mut bool) {
    let catalogue = draft.catalogue.clone();
    kit::section(ui, "Wheels");
    kit::toggle(ui, "Same front and rear", same);
    if *same {
        draft.wheels[1] = draft.wheels[0];
    }
    let axles: &[usize] = if *same { &[0] } else { &[0, 1] };
    for &axle in axles {
        let wheel = &mut draft.wheels[axle];
        let place = if *same {
            "All round"
        } else {
            ["Front", "Rear"][axle]
        };
        kit::info(
            ui,
            place,
            &format!(
                "{}: {:.0} mm across, {:.1} kg",
                wheel.tire.label(),
                2000.0 * wheel.tire.radius(),
                wheel.mass()
            ),
        );
        wheel_picker(ui, &catalogue, wheel);
    }
    if *same {
        draft.wheels[1] = draft.wheels[0];
    }
    if let Some(why) = draft.wheel_misfit() {
        kit::warning(ui, &why);
    }
    kit::note(
        ui,
        "Wider tires grip more but weigh more and roll heavier; bigger wheels spin up \
         slower, and taller tires gear the car longer.",
    );
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

/// Rim diameter and width, tire width and sidewall, and compound. Another rim diameter or
/// tire width keeps the outside diameter as near as it goes, on a rim that suits the tire.
fn wheel_picker(ui: &mut egui::Ui, catalogue: &Catalogue, wheel: &mut Wheel) {
    let diameter = 2.0 * wheel.tire.radius();
    let resize = |wheel: &mut Wheel| {
        wheel.tire = TireSize::nearest(wheel.tire.width, wheel.rim.diameter, diameter);
        wheel.rim.width = wheel.tire.rim_width_near(wheel.rim.width);
    };
    let rims: Vec<_> = wheels::RIM_DIAMETERS.collect();
    if kit::pick(ui, "Rim diameter", &mut wheel.rim.diameter, &rims, |d| {
        format!("{d}\"")
    }) {
        resize(wheel);
    }
    let widths: Vec<_> = wheels::RIM_WIDTHS.step_by(wheels::RIM_WIDTH_STEP).collect();
    kit::pick(ui, "Rim width", &mut wheel.rim.width, &widths, |w| {
        format!("{:.1}\"", f32::from(w) / 10.0)
    });
    let widths: Vec<_> = wheels::TIRE_WIDTHS
        .step_by(wheels::TIRE_WIDTH_STEP)
        .collect();
    if kit::pick(ui, "Tire width", &mut wheel.tire.width, &widths, |w| {
        format!("{w} mm")
    }) {
        resize(wheel);
    }
    let aspects: Vec<_> = wheels::ASPECTS.step_by(wheels::ASPECT_STEP).collect();
    kit::pick(ui, "Sidewall", &mut wheel.tire.aspect, &aspects, |a| {
        format!("{a} %")
    });
    let compounds: Vec<usize> = (0..catalogue.compounds.len()).collect();
    kit::pick(ui, "Compound", &mut wheel.compound, &compounds, |c| {
        catalogue.compounds[c].name.clone()
    });
    kit::note(ui, &catalogue.compounds[wheel.compound].about);
}

/// Which engine: any that fits the chassis' bay.
fn engine(ui: &mut egui::Ui, garage: &mut Garage) {
    let (build, draft) = &mut garage.draft;
    let catalogue = build.catalogue.clone();
    let chassis = build.chassis();
    kit::section(ui, "Engine");
    // Every version that fits, as the factory builds it in this car.
    let fitting: Vec<(usize, usize)> = (catalogue.families.iter())
        .enumerate()
        .filter(|(f, _)| chassis.fit(&catalogue, *f).is_ok())
        .flat_map(|(f, family)| (0..family.versions.len()).map(move |v| (f, v)))
        .collect();
    let mut chosen = (draft.family, draft.version);
    let name = |(f, v): (usize, usize)| {
        let version = &catalogue.families[f].versions[v];
        format!("{} {}", version.code, version.name)
    };
    if kit::pick(ui, "Engine", &mut chosen, &fitting, name)
        && let Ok(parts) = build.install(Parts::of_version(&catalogue, chosen.0, chosen.1))
    {
        *draft = parts;
    }
    if kit::button(ui, "Factory parts for it").clicked()
        && let Ok(parts) = build.install(Parts::of_version(&catalogue, draft.family, draft.version))
    {
        *draft = parts;
    }
    let family = draft.family();
    kit::note(ui, &family.about);
    kit::info(
        ui,
        &format!("{} bay", chassis.bay.place),
        &format!(
            "{:.0} × {:.0} × {:.0} cm",
            chassis.bay.length * 100.0,
            chassis.bay.width * 100.0,
            chassis.bay.height * 100.0,
        ),
    );
    let others = catalogue.families.len()
        - (0..catalogue.families.len())
            .filter(|&f| chassis.fit(&catalogue, f).is_ok())
            .count();
    if others > 0 {
        kit::note(
            ui,
            &format!(
                "{others} engine families don't fit in the {}: the bay's room is all that \
                 limits a swap.",
                chassis.name
            ),
        );
    }
    match chassis.fit(&catalogue, draft.family) {
        Ok(fit) => {
            if fit.swap {
                kit::note(
                    ui,
                    &format!(
                        "The {} needs a swap subframe in the {}.",
                        family.name, chassis.name
                    ),
                );
            }
            if catalogue.needs_adapter(build.gearbox, draft.family) {
                kit::note(
                    ui,
                    &format!(
                        "The {} gearbox needs an adapter plate for it.",
                        build.gearbox().name
                    ),
                );
            }
        }
        Err(misfit) => {
            kit::warning(ui, &format!("The {} is {}.", family.name, misfit.0));
        }
    }
}

/// The engine's parts to choose from, a row per slot.
fn parts(ui: &mut egui::Ui, garage: &mut Garage) {
    let draft = &mut garage.draft.1;
    let catalogue = draft.catalogue.clone();
    kit::section(ui, "Engine parts");
    for (i, slot) in catalogue.slots.iter().enumerate() {
        let enabled = !slot.needs_turbo || draft.has_turbo();
        let labels: Vec<String> = (0..slot.parts.len())
            .map(|part| draft.label(i, part))
            .collect();
        let chosen = &mut draft.chosen[i];
        ui.add_enabled_ui(enabled, |ui| {
            kit::choice(ui, &slot.name, chosen, &labels);
        });
        if slot.readout == Readout::Headers && draft.header_room.is_finite() {
            kit::note(
                ui,
                &format!(
                    "Room for {:.0} mm primaries around this engine: longer ones are made \
                     shorter.",
                    draft.header_room * 1e3
                ),
            );
        }
    }
    kit::pick(ui, "ECU", &mut draft.ecu, &EcuModel::ALL, |model| {
        model.name().to_string()
    });
    kit::note(
        ui,
        "Stock ECU: fuel, spark, rev limiter and idle. Programmable: also switches cams and \
         drives the cam phasers.",
    );
}

/// What the parts make, and building it: true when "Build" was chosen.
fn build(ui: &mut egui::Ui, garage: &mut Garage, car: &Car) -> bool {
    let (draft, parts) = &garage.draft;
    let hardware = parts.hardware();
    let current = &car.setup.engine.build;
    let rebake = maps::fingerprint(&hardware) != maps::fingerprint(&current.hardware);
    let new_engine = *parts != car.setup.engine.parts;
    let changed = new_engine || *draft != car.setup.build;
    let mut build_now = false;
    if let Some(job) = &garage.job {
        match &job.work {
            Some(work) => kit::progress(ui, "Baking the maps…", work.progress()),
            None => kit::progress(ui, "Getting the maps…", 0.0),
        }
        if kit::button(ui, "Cancel").clicked() {
            if let Some(work) = garage.job.take().and_then(|job| job.work) {
                work.cancel();
            }
            garage.status = "Cancelled.".to_string();
        }
    } else {
        let fits = garage.draft.0.wheel_misfit().is_none();
        build_now = ui
            .add_enabled_ui(changed && fits, |ui| kit::button(ui, "Build it"))
            .inner
            .clicked();
        let revert = ui
            .add_enabled_ui(changed, |ui| kit::button(ui, "Back to how it is"))
            .inner
            .clicked();
        if revert {
            garage.draft = (car.setup.build.clone(), car.setup.engine.parts.clone());
        }
    }
    kit::note(
        ui,
        if !changed {
            "This is the car as it's built."
        } else if !new_engine {
            "The engine stays as it is."
        } else if rebake && store::is_stock(&hardware) {
            "A factory engine: it fits right away."
        } else if rebake {
            "The cylinder side changes: its maps need baking (a few seconds), unless it was \
             built before."
        } else {
            "Only bolt-on parts change: it fits right away."
        },
    );
    if !garage.status.is_empty() {
        kit::note(ui, &garage.status);
    }
    kit::toggle(ui, "Factory tune for the new parts", &mut garage.retune);
    let (_, parts) = &garage.draft;
    if hardware.has_cam_control() && !parts.ecu.cam_control() {
        kit::warning(
            ui,
            "The stock ECU can't drive the switching cam or the phasers: they stay on the \
             mild profile, at rest.",
        );
    }
    kit::info(
        ui,
        &hardware.name,
        &format!(
            "{:.0} cm³, {:.1}:1, redline {:.0}",
            hardware.displacement() * 1e6,
            hardware.compression_ratio,
            hardware.redline(),
        ),
    );
    let family = parts.family();
    kit::info(
        ui,
        family.layout.name(),
        &format!(
            "{:.0} × {:.0} × {:.0} cm, {:.0} kg",
            family.length * 100.0,
            family.width * 100.0,
            family.height * 100.0,
            family.built_mass(&hardware),
        ),
    );
    build_now
}

/// How worn the engine's parts are, what has failed, and repairing it; and new tires.
fn condition(ui: &mut egui::Ui, car: &mut Car) {
    let cylinders = car.setup.engine.build.hardware.cylinders as usize;
    let damage = &car.drivetrain.engine.damage;
    kit::section(ui, "Condition");
    let worn: Vec<String> = damage
        .parts(cylinders)
        .into_iter()
        .filter(|(_, wear)| *wear >= 0.005)
        .take(4)
        .map(|(name, wear)| format!("{name} {:.0} %", wear * 100.0))
        .collect();
    kit::note(
        ui,
        &if worn.is_empty() {
            "The engine is like new.".to_string()
        } else {
            format!("Worn: {}", worn.join(", "))
        },
    );
    for failure in damage.failures(cylinders) {
        kit::warning(ui, &failure);
    }
    let broken = damage.worst(cylinders) > 0.0;
    if ui
        .add_enabled_ui(broken, |ui| kit::button(ui, "Repair the engine"))
        .inner
        .clicked()
    {
        car.drivetrain.engine.damage = Default::default();
    }
    let worn = (car.corners.iter())
        .map(|corner| corner.tread.wear())
        .fold(0.0, f32::max);
    let tires = format!("worn {:.0} %", worn * 100.0);
    if kit::button_with(ui, "New tires (the same, new and cold)", &tires).clicked() {
        for corner in &mut car.corners {
            corner.tread = Tread::default();
        }
    }
}

/// Builds the draft: right away if the engine's maps are at hand (the car's own, the
/// factory's or cached ones), else once they come from the server or are baked (`bake`).
fn start(
    commands: &mut Commands,
    terrain: &Terrain,
    garage: &mut Garage,
    entity: Entity,
    car: &mut Car,
) {
    let (build, parts) = garage.draft.clone();
    if let Some(why) = build.wheel_misfit() {
        garage.status = format!("{why}.");
        return;
    }
    let parts = match build.install(parts) {
        Ok(parts) => parts,
        Err(misfit) => {
            garage.status = format!("The {} is {}.", garage.draft.1.family().name, misfit.0);
            return;
        }
    };
    garage.draft.1 = parts.clone();
    let hardware = parts.hardware();
    let fingerprint = maps::fingerprint(&hardware);
    let current = &car.setup.engine.build;
    let at_hand = if fingerprint == maps::fingerprint(&current.hardware) {
        Some(current.maps.clone())
    } else {
        workshop::at_hand(&hardware)
    };
    if let Some(maps) = at_hand {
        let made = (build, parts, hardware, maps);
        fit(commands, terrain, entity, car, made, garage.retune);
        garage.status = "Built.".to_string();
        return;
    }
    garage.status.clear();
    garage.job = Some(Job {
        car: car.index,
        build,
        parts,
        work: None,
        hardware,
        retune: garage.retune,
    });
}

/// A chassis' page in the showroom: what it is, its engines and paint, and the button
/// to drive it (true when clicked).
fn brochure(ui: &mut egui::Ui, catalogue: &Shared, shown: &Showroom) {
    let chassis = &catalogue.chassis[shown.chassis];
    let body = &chassis.body;
    kit::section(ui, &chassis.class);
    kit::note(ui, &chassis.about);
    let build = CarBuild::factory_in(catalogue, shown.chassis);
    let parts = build.factory_engine(shown.engine);
    kit::note(ui, &parts.family().about);
    let gearbox = build.gearbox();
    let mass = chassis.mass
        + parts.family().built_mass(&parts.hardware())
        + gearbox.mass
        + 2.0 * (chassis.unsprung(0) + chassis.unsprung(1));
    kit::info(ui, "Engine", &format!("at the {}", chassis.bay.place));
    kit::info(ui, "Drive", chassis.drive.name());
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
    let [front, rear] = chassis.wheels.map(|wheel| catalogue.wheel_label(&wheel));
    if front == rear {
        kit::info(ui, "Tires", &front);
    } else {
        kit::info(ui, "Tires", &format!("{front} / {rear}"));
    }
    kit::info(ui, "Paint", &catalogue.paints[chassis.paint].name);
    kit::note(
        ui,
        "A parts shop fits any engine there's room for; any paint shop changes the paint.",
    );
}
