//! The showroom (F6) and the garage (F4). The showroom has every car as it leaves the
//! factory; the garage builds the active car from parts: what's fitted to the chassis
//! (paint, gearbox, clutch, final drive, differentials, suspension, tires), which engine
//! (any that fits the engine bay), and the engine's own parts.
//!
//! Parts on the cylinder side need the engine's maps baked again, which takes a few
//! seconds: on a thread, or on the web a slice every frame, while the car keeps its old
//! engine. Baked maps are cached (`~/.cache/car-sim/engines`, local storage on the web),
//! so going back to a build is quick.

#[cfg(target_arch = "wasm32")]
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::car::{ActiveCar, Car, CarSetup, heading};
use crate::chassis::{
    CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, SUSPENSIONS, TIRES, needs_adapter,
};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::families::{FAMILIES, Version};
use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, Bake, EngineMaps};
use crate::physics::engine::parts::{HEADERS, INTERCOOLER, Parts, SLOTS};
use crate::storage::Folder;
use crate::terrain::Terrain;

const MAPS: Folder = Folder {
    cache: true,
    name: "engines",
    extension: "maps",
    web_prefix: "car-sim-maps:",
};

/// Grid points baked per batch on the thread, and time spent baking per frame on the web.
#[cfg(not(target_arch = "wasm32"))]
const BATCH: usize = 64;
#[cfg(target_arch = "wasm32")]
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(12);

const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 170, 60);

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Garage>()
            .add_systems(Update, (toggle, bake))
            .add_systems(
                EguiPrimaryContextPass,
                (
                    window.run_if(|garage: Res<Garage>| garage.open),
                    showroom.run_if(|garage: Res<Garage>| garage.showroom.is_some()),
                ),
            );
    }
}

#[derive(Resource)]
struct Garage {
    open: bool,
    /// The showroom, if it's open: the chassis looked at, and the engine option and paint
    /// picked for it.
    showroom: Option<Showroom>,
    /// The car the draft was taken from, and what's picked so far: the car's build and
    /// its engine's parts.
    car: Option<usize>,
    draft: (CarBuild, Parts),
    /// Give the new engine a factory tune, rather than keep the ECU's tune.
    retune: bool,
    job: Option<Job>,
    status: String,
}

struct Showroom {
    chassis: usize,
    engine: usize,
    paint: usize,
}

impl Default for Garage {
    fn default() -> Self {
        let stock = CarSetup::default();
        Self {
            open: false,
            showroom: None,
            car: None,
            draft: (stock.build, stock.engine.parts),
            retune: true,
            job: None,
            status: String::new(),
        }
    }
}

/// An engine being built: for which car, from what, and its maps baking.
struct Job {
    car: usize,
    build: CarBuild,
    parts: Parts,
    hardware: EngineHardware,
    retune: bool,
    work: Work,
}

/// Baking on a thread, which reports its progress (an f32's bits) and can be told to stop.
#[cfg(not(target_arch = "wasm32"))]
struct Work {
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Option<EngineMaps>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Work {
    fn start(hardware: &EngineHardware) -> Self {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut bake = Bake::new(hardware);
        let (shown, stop) = (progress.clone(), cancel.clone());
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(maps) = bake.advance(BATCH) {
                    return Some(maps);
                }
                shown.store(bake.progress().to_bits(), Ordering::Relaxed);
            }
            None
        });
        Self {
            progress,
            cancel,
            thread: Some(thread),
        }
    }

    fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// The maps once they're done, or nothing if baking failed.
    fn poll(&mut self) -> Option<Option<EngineMaps>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().ok().flatten())
    }

    fn cancel(self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Baking a slice every frame (there are no threads on the web).
#[cfg(target_arch = "wasm32")]
struct Work {
    bake: Bake,
}

#[cfg(target_arch = "wasm32")]
impl Work {
    fn start(hardware: &EngineHardware) -> Self {
        Self {
            bake: Bake::new(hardware),
        }
    }

    fn progress(&self) -> f32 {
        self.bake.progress()
    }

    fn poll(&mut self) -> Option<Option<EngineMaps>> {
        let start = Instant::now();
        while start.elapsed() < FRAME_BUDGET {
            if let Some(maps) = self.bake.advance(1) {
                return Some(Some(maps));
            }
        }
        None
    }

    fn cancel(self) {}
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut garage: ResMut<Garage>) {
    if keys.just_pressed(KeyCode::F4) {
        garage.open = !garage.open;
        // Take the parts from the car again when it opens.
        garage.car = None;
    }
    if keys.just_pressed(KeyCode::F6) {
        garage.showroom = match garage.showroom {
            Some(_) => None,
            None => Some(Showroom {
                chassis: garage.draft.0.chassis,
                engine: 0,
                paint: garage.draft.0.paint,
            }),
        };
    }
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
    mut cars: Query<(Entity, &mut Car)>,
) {
    let Some(job) = &mut garage.job else {
        return;
    };
    let Some(baked) = job.work.poll() else {
        return;
    };
    let job = garage.job.take().expect("a job");
    let Some(maps) = baked else {
        garage.status = "Baking the maps failed.".to_string();
        return;
    };
    let _ = MAPS.save(&cache_name(&job.hardware), &maps.to_text(&job.hardware));
    garage.status = match active(&mut cars, job.car) {
        Some((entity, mut car)) => {
            let made = (job.build, job.parts, job.hardware, maps);
            fit(&mut commands, &terrain, entity, &mut car, made, job.retune);
            "Built.".to_string()
        }
        None => "The car is gone.".to_string(),
    };
}

fn cache_name(hardware: &EngineHardware) -> String {
    format!("{:016x}", maps::fingerprint(hardware))
}

/// Rebuilds `car` as `build` with the engine built from `parts`. An engine of other
/// parts is new (no wear, at rest), with a factory tune or the tune it had (moved over to
/// a different ECU, which takes the factory's cam control if the old one had none). A
/// car that only changed under the skin carries on as it moves; one with another chassis,
/// paint or wheels is put down afresh where the old one was.
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
    commands.entity(entity).despawn();
    commands.spawn((Transform::from_translation(new.body.position), new));
}

fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    active_car: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    mut garage: ResMut<Garage>,
) -> Result {
    let Some((entity, mut car)) = active(&mut cars, active_car.0) else {
        return Ok(());
    };
    if garage.car != Some(car.index) || garage.draft.0.chassis != car.setup.build.chassis {
        garage.car = Some(car.index);
        garage.draft = (car.setup.build.clone(), car.setup.engine.parts.clone());
    }
    let mut open = garage.open;
    let mut build_now = false;
    egui::Window::new("Garage")
        .open(&mut open)
        .default_pos([360.0, 10.0])
        .vscroll(true)
        .show(contexts.ctx_mut()?, |ui| {
            car_parts(ui, &mut garage.draft.0);
            ui.separator();
            engine(ui, &mut garage);
            parts(ui, &mut garage);
            ui.separator();
            build_now = build(ui, &mut garage, &car);
            ui.separator();
            condition(ui, &mut car);
        });
    garage.open = open;
    if build_now {
        start(&mut commands, &terrain, &mut garage, entity, &mut car);
    }
    Ok(())
}

/// What's fitted to the chassis.
fn car_parts(ui: &mut egui::Ui, draft: &mut CarBuild) {
    let chassis = draft.chassis();
    ui.horizontal(|ui| {
        ui.strong(format!(
            "{} ({})",
            chassis.name,
            chassis.class.to_lowercase()
        ));
        ui.label("F6: showroom");
    });
    let combo = |ui: &mut egui::Ui, id: &str, chosen: &mut usize, names: &[&str]| {
        egui::ComboBox::from_id_salt(id)
            .width(220.0)
            .selected_text(names[*chosen])
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    ui.selectable_value(chosen, i, *name);
                }
            });
    };
    egui::Grid::new("car").num_columns(2).show(ui, |ui| {
        ui.label("Paint");
        combo(ui, "paint", &mut draft.paint, &PAINTS.map(|(name, _)| name));
        ui.end_row();
        ui.label("Gearbox")
            .on_hover_text("Any gearbox made for the chassis' layout");
        egui::ComboBox::from_id_salt("gearbox")
            .width(220.0)
            .selected_text(GEARBOXES[draft.gearbox].name)
            .show_ui(ui, |ui| {
                for gearbox in chassis.gearboxes() {
                    ui.selectable_value(&mut draft.gearbox, gearbox, GEARBOXES[gearbox].name);
                }
            });
        ui.end_row();
        ui.label("Clutch").on_hover_text(
            "An engine with more torque than the clutch passes makes it slip: the revs run \
             away from the wheels",
        );
        let passes = |clutch: usize| {
            let (name, more) = CLUTCHES[clutch];
            let torque = GEARBOXES[draft.gearbox].clutch_torque * more;
            format!("{name} ({torque:.0} N·m)")
        };
        egui::ComboBox::from_id_salt("clutch")
            .width(220.0)
            .selected_text(passes(draft.clutch))
            .show_ui(ui, |ui| {
                for clutch in 0..CLUTCHES.len() {
                    ui.selectable_value(&mut draft.clutch, clutch, passes(clutch));
                }
            });
        ui.end_row();
        ui.label("Final drive")
            .on_hover_text("Shorter (a higher number) pulls harder and tops out sooner");
        ui.add(
            egui::Slider::new(&mut draft.final_drive, 2.5..=5.5)
                .step_by(0.05)
                .suffix(":1"),
        );
        ui.end_row();
        ui.label("Differentials")
            .on_hover_text("Open differentials let a wheel without grip spin away the torque");
        ui.checkbox(&mut draft.limited_slip, "Limited-slip");
        ui.end_row();
        ui.label("Suspension")
            .on_hover_text("Stiffer springs and dampers, and more negative camber");
        combo(
            ui,
            "suspension",
            &mut draft.suspension,
            &SUSPENSIONS.map(|kit| kit.0),
        );
        ui.end_row();
        ui.label("Tires");
        combo(ui, "tires", &mut draft.tires, &TIRES.map(|(name, _)| name));
        ui.end_row();
    });
}

/// Which engine: any that fits the chassis' bay.
fn engine(ui: &mut egui::Ui, garage: &mut Garage) {
    let (build, draft) = &mut garage.draft;
    let chassis = build.chassis();
    ui.horizontal(|ui| {
        ui.label("Engine:").on_hover_text(draft.family().about);
        let name = |version: &Version| format!("{} {}", version.code, version.name);
        let current = &draft.family().versions[draft.version];
        egui::ComboBox::from_id_salt("engine")
            .width(220.0)
            .height(500.0)
            .selected_text(name(current))
            .show_ui(ui, |ui| {
                for (f, family) in FAMILIES.iter().enumerate() {
                    let fit = chassis.fit(f);
                    for (v, version) in family.versions.iter().enumerate() {
                        let chosen = (draft.family, draft.version) == (f, v);
                        let label = egui::Button::selectable(chosen, name(version));
                        match &fit {
                            Ok(_) => {
                                let pick = ui.add(label).on_hover_text(family.about);
                                // As the factory builds it, in this car.
                                if let (true, Ok(parts)) =
                                    (pick.clicked(), build.install(Version::parts(f, v)))
                                {
                                    *draft = parts;
                                }
                            }
                            Err(misfit) => {
                                ui.add_enabled(false, label)
                                    .on_disabled_hover_text(&misfit.0);
                            }
                        }
                    }
                }
            });
        if ui.button("Factory parts").clicked()
            && let Ok(parts) = build.install(Version::parts(draft.family, draft.version))
        {
            *draft = parts;
        }
    });
    let family = draft.family();
    ui.label(format!(
        "{} bay: {:.0} × {:.0} × {:.0} cm",
        chassis.bay.place,
        chassis.bay.length * 100.0,
        chassis.bay.width * 100.0,
        chassis.bay.height * 100.0,
    ))
    .on_hover_text("Along the car, across it, and up: what fits in is all that limits a swap");
    match chassis.fit(draft.family) {
        Ok(fit) => {
            if fit.swap {
                ui.label(format!(
                    "The {} needs a swap subframe in the {}.",
                    family.name, chassis.name
                ));
            }
            if needs_adapter(build.gearbox, draft.family) {
                ui.label(format!(
                    "The {} gearbox needs an adapter plate for it.",
                    GEARBOXES[build.gearbox].name
                ));
            }
        }
        Err(misfit) => {
            ui.colored_label(WARNING, format!("The {} is {}.", family.name, misfit.0));
        }
    }
}

/// The engine's parts to choose from, a row per slot.
fn parts(ui: &mut egui::Ui, garage: &mut Garage) {
    let draft = &mut garage.draft.1;
    egui::Grid::new("parts").num_columns(2).show(ui, |ui| {
        for (i, slot) in SLOTS.iter().enumerate() {
            ui.label(slot.name).on_hover_text(slot.about);
            let enabled = i != INTERCOOLER || draft.has_turbo();
            let labels: Vec<String> = (0..slot.parts.len())
                .map(|part| draft.label(i, part))
                .collect();
            let chosen = &mut draft.chosen[i];
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt(slot.name)
                    .width(220.0)
                    .selected_text(&labels[*chosen])
                    .show_ui(ui, |ui| {
                        for (j, label) in labels.iter().enumerate() {
                            ui.selectable_value(chosen, j, label);
                        }
                    });
            });
            ui.end_row();
            if i == HEADERS && draft.header_room.is_finite() {
                ui.label("");
                ui.label(format!(
                    "(room for {:.0} mm primaries around this engine)",
                    draft.header_room * 1e3
                ))
                .on_hover_text(
                    "Longer primaries than fit beside the engine in this car are made \
                     shorter",
                );
                ui.end_row();
            }
        }
        ui.label("Cam timing")
            .on_hover_text("Adjustable cam gears: advancing the intake cam fills better low down");
        ui.vertical(|ui| {
            ui.add(degrees(&mut draft.intake_timing, "intake advance"));
            ui.add(degrees(&mut draft.exhaust_timing, "exhaust retard"));
        });
        ui.end_row();
        ui.label("Wastegate")
            .on_hover_text("The boost where the wastegate starts to open");
        let turbo = draft.has_turbo();
        ui.add_enabled(
            turbo,
            egui::Slider::new(&mut draft.boost, 0.3..=1.5)
                .step_by(0.05)
                .suffix(" bar"),
        );
        ui.end_row();
        ui.label("ECU").on_hover_text(
            "Stock: fuel, spark, rev limiter and idle. Programmable: also switches cams and \
             drives the cam phasers",
        );
        egui::ComboBox::from_id_salt("ecu")
            .width(220.0)
            .selected_text(draft.ecu.name())
            .show_ui(ui, |ui| {
                for model in EcuModel::ALL {
                    ui.selectable_value(&mut draft.ecu, model, model.name());
                }
            });
        ui.end_row();
    });
}

/// A slider for a cam's timing (crank degrees).
fn degrees<'a>(value: &'a mut f32, text: &str) -> egui::Slider<'a> {
    egui::Slider::new(value, -10.0..=10.0)
        .step_by(1.0)
        .suffix("°")
        .text(text)
}

/// What the parts make, and the button to build it: true when it was clicked.
fn build(ui: &mut egui::Ui, garage: &mut Garage, car: &Car) -> bool {
    let (draft, parts) = &garage.draft;
    let hardware = parts.hardware();
    let current = &car.setup.engine.build;
    let rebake = maps::fingerprint(&hardware) != maps::fingerprint(&current.hardware);
    ui.label(format!(
        "{}: {:.0} cm³, {:.1}:1, redline {:.0} rpm",
        hardware.name,
        hardware.displacement() * 1e6,
        hardware.compression_ratio,
        hardware.redline(),
    ))
    .on_hover_text("Where the valves would soon float, or the rods' load near its limit");
    let family = parts.family();
    ui.label(format!(
        "{}: {}, {:.0} × {:.0} × {:.0} cm, {:.0} kg",
        family.name,
        family.layout.name(),
        family.length * 100.0,
        family.width * 100.0,
        family.height * 100.0,
        family.built_mass(&hardware),
    ))
    .on_hover_text("Length along the crank, width and height: what it takes to fit it");
    let new_engine = *parts != car.setup.engine.parts;
    let changed = new_engine || *draft != car.setup.build;
    ui.label(if !changed {
        "This is the car as it's built."
    } else if !new_engine {
        "The engine stays as it is."
    } else if rebake {
        "The cylinder side changes: its maps need baking (a few seconds)."
    } else {
        "Only bolt-on parts change: it fits right away."
    });
    if hardware.has_cam_control() && !parts.ecu.cam_control() {
        ui.colored_label(
            WARNING,
            "The stock ECU can't drive the switching cam or the phasers: they stay on the \
             mild profile, at rest.",
        );
    }
    ui.checkbox(&mut garage.retune, "Factory tune for the new parts")
        .on_hover_text("Otherwise the ECU keeps its tune, made for the old parts");

    let mut build_now = false;
    if let Some(progress) = garage.job.as_ref().map(|job| job.work.progress()) {
        ui.horizontal(|ui| {
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_width(200.0)
                    .text("Baking the maps…"),
            );
            if ui.button("Cancel").clicked() {
                garage.job.take().expect("a job").work.cancel();
                garage.status = "Cancelled.".to_string();
            }
        });
    } else {
        ui.horizontal(|ui| {
            build_now = ui
                .add_enabled(changed, egui::Button::new("Build"))
                .clicked();
            if ui
                .add_enabled(changed, egui::Button::new("Revert"))
                .clicked()
            {
                garage.draft = (car.setup.build.clone(), car.setup.engine.parts.clone());
            }
        });
    }
    if !garage.status.is_empty() {
        ui.label(&garage.status);
    }
    build_now
}

/// How worn the engine's parts are, what has failed, and repairing it.
fn condition(ui: &mut egui::Ui, car: &mut Car) {
    let cylinders = car.setup.engine.build.hardware.cylinders as usize;
    let damage = &car.drivetrain.engine.damage;
    ui.horizontal(|ui| {
        ui.label("Condition:");
        let worn: Vec<String> = damage
            .parts(cylinders)
            .into_iter()
            .filter(|(_, wear)| *wear >= 0.005)
            .take(4)
            .map(|(name, wear)| format!("{name} {:.0} %", wear * 100.0))
            .collect();
        ui.label(if worn.is_empty() {
            "like new".to_string()
        } else {
            worn.join(", ")
        });
    });
    for failure in damage.failures(cylinders) {
        ui.colored_label(egui::Color32::from_rgb(255, 90, 70), failure);
    }
    let repair = egui::Button::new("Repair");
    if ui
        .add_enabled(damage.worst(cylinders) > 0.0, repair)
        .clicked()
    {
        car.drivetrain.engine.damage = Default::default();
    }
}

/// Builds the draft: right away if the engine's maps are at hand (the car's own, the
/// factory's or cached ones), else starts baking them.
fn start(
    commands: &mut Commands,
    terrain: &Terrain,
    garage: &mut Garage,
    entity: Entity,
    car: &mut Car,
) {
    let (build, parts) = garage.draft.clone();
    let parts = match build.install(parts) {
        Ok(parts) => parts,
        Err(misfit) => {
            garage.status = format!(
                "The {} is {}.",
                FAMILIES[garage.draft.1.family].name, misfit.0
            );
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
        EngineMaps::stock(&hardware).or_else(|| {
            let text = MAPS.load(&cache_name(&hardware))?;
            EngineMaps::from_text(&text, fingerprint)
        })
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
        work: Work::start(&hardware),
        hardware,
        retune: garage.retune,
    });
}

/// Every car as it leaves the factory, to pick one and drive it.
fn showroom(
    mut commands: Commands,
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    active_car: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    mut garage: ResMut<Garage>,
) -> Result {
    let Some((entity, mut car)) = active(&mut cars, active_car.0) else {
        return Ok(());
    };
    let Some(mut shown) = garage.showroom.take() else {
        return Ok(());
    };
    let (mut open, mut drive) = (true, false);
    egui::Window::new("Showroom")
        .open(&mut open)
        .default_pos([200.0, 60.0])
        .default_width(560.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    for (i, chassis) in CHASSIS.iter().enumerate() {
                        let label = format!("{}  ·  {}", chassis.name, chassis.class);
                        if ui.selectable_label(shown.chassis == i, label).clicked() {
                            shown = Showroom {
                                chassis: i,
                                engine: 0,
                                paint: chassis.paint,
                            };
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(330.0);
                    drive = brochure(ui, &mut shown);
                });
            });
        });
    if drive {
        let mut build = CarBuild::factory(shown.chassis);
        build.paint = shown.paint;
        let parts = build.factory_engine(shown.engine);
        garage.draft = (build, parts);
        garage.car = Some(car.index);
        garage.retune = true;
        start(&mut commands, &terrain, &mut garage, entity, &mut car);
    }
    garage.showroom = (open && !drive).then_some(shown);
    Ok(())
}

/// A chassis' page in the showroom: what it is, its engines and paints, and the button
/// to drive it (true when clicked).
fn brochure(ui: &mut egui::Ui, shown: &mut Showroom) -> bool {
    let chassis = &CHASSIS[shown.chassis];
    let body = &chassis.body;
    ui.heading(chassis.name);
    ui.label(chassis.class);
    ui.label(chassis.about);
    ui.add_space(4.0);
    ui.label(format!(
        "Engine at the {}, {}; {}",
        chassis.bay.place,
        chassis.drive.name(),
        GEARBOXES[chassis.gearbox].name.to_lowercase(),
    ));
    ui.label(format!(
        "{:.2} m long, {:.2} m wide, {:.2} m tall, wheelbase {:.2} m",
        body.length,
        body.width,
        body.height(),
        chassis.wheelbase,
    ));
    ui.add_space(4.0);
    ui.strong("Engine");
    for (option, code) in chassis.engines.iter().enumerate() {
        let Some(parts) = Parts::factory(code) else {
            continue;
        };
        let name = parts.hardware().name;
        ui.radio_value(&mut shown.engine, option, name)
            .on_hover_text(parts.family().about);
    }
    let build = CarBuild::factory(shown.chassis);
    let parts = build.factory_engine(shown.engine);
    let mass = chassis.mass
        + parts.family().built_mass(&parts.hardware())
        + GEARBOXES[chassis.gearbox].mass
        + 2.0 * (chassis.unsprung[0] + chassis.unsprung[1]);
    ui.label(format!("{mass:.0} kg with it"));
    ui.label("(F4, the garage, fits any engine there's room for)");
    ui.add_space(4.0);
    ui.strong("Paint");
    ui.horizontal_wrapped(|ui| {
        for (i, (name, [r, g, b])) in PAINTS.iter().enumerate() {
            let color =
                egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
            let swatch = egui::Button::new("    ")
                .fill(color)
                .selected(shown.paint == i);
            if ui.add(swatch).on_hover_text(*name).clicked() {
                shown.paint = i;
            }
        }
    });
    ui.add_space(8.0);
    ui.button("Drive it").clicked()
}
