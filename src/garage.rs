//! The garage (F4): building the active car's engine from parts. Parts on the cylinder
//! side need the engine's maps baked again, which takes a few seconds: on a thread, or on
//! the web a slice every frame, while the car keeps its old engine. Baked maps are cached
//! (`~/.cache/car-sim/engines`, local storage on the web), so going back to a build is
//! quick.

#[cfg(target_arch = "wasm32")]
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::car::{ActiveCar, Car};
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, Bake, EngineMaps};
use crate::physics::engine::parts::{INTERCOOLER, Parts, SLOTS};
use crate::physics::engine::{Engine, EngineBuild};
use crate::storage::Folder;

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

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Garage>()
            .add_systems(Update, (toggle, bake))
            .add_systems(
                EguiPrimaryContextPass,
                window.run_if(|garage: Res<Garage>| garage.open),
            );
    }
}

#[derive(Resource)]
struct Garage {
    open: bool,
    /// The car the draft was taken from, and the parts picked so far.
    car: Option<usize>,
    draft: Parts,
    /// Give the new engine a factory tune, rather than keep the ECU's tune.
    retune: bool,
    job: Option<Job>,
    status: String,
}

impl Default for Garage {
    fn default() -> Self {
        Self {
            open: false,
            car: None,
            draft: Parts::stock(),
            retune: true,
            job: None,
            status: String::new(),
        }
    }
}

/// An engine being built: for which car, from what, and its maps baking.
struct Job {
    car: usize,
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
}

/// Moves the baking job on, and fits the engine to its car when the maps are done.
fn bake(mut garage: ResMut<Garage>, mut cars: Query<&mut Car>) {
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
    garage.status = match cars.iter_mut().find(|car| car.index == job.car) {
        Some(mut car) => {
            fit(&mut car, job.parts, job.hardware, maps, job.retune);
            "Built.".to_string()
        }
        None => "The car is gone.".to_string(),
    };
}

fn cache_name(hardware: &EngineHardware) -> String {
    format!("{:016x}", maps::fingerprint(hardware))
}

/// Puts the engine built from `parts` into `car`, as new (no wear, at rest), with a
/// factory tune or the tune it had (moved over to a different ECU, which takes the
/// factory's cam control if the old one had none).
fn fit(car: &mut Car, parts: Parts, hardware: EngineHardware, maps: EngineMaps, retune: bool) {
    let build = &car.setup.engine.build;
    let factory = || Ecu::calibrated(&hardware, &maps, 95.0, parts.ecu);
    let ecu = if retune {
        factory()
    } else {
        let mut ecu = build.ecu.clone();
        if parts.ecu.cam_control() && !ecu.model.cam_control() {
            ecu.cams = factory().cams;
        }
        ecu.model = parts.ecu;
        ecu
    };
    car.setup.engine.build = EngineBuild {
        hardware,
        maps,
        ecu,
        octane: build.octane,
    };
    car.setup.engine.parts = parts;
    car.drivetrain.engine = Engine::default();
}

fn window(
    mut contexts: EguiContexts,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut garage: ResMut<Garage>,
) -> Result {
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    if garage.car != Some(car.index) {
        garage.car = Some(car.index);
        garage.draft = car.setup.engine.parts.clone();
    }
    let mut open = garage.open;
    egui::Window::new("Garage")
        .open(&mut open)
        .default_pos([360.0, 10.0])
        .show(contexts.ctx_mut()?, |ui| {
            parts(ui, &mut garage);
            ui.separator();
            build(ui, &mut garage, &mut car);
            ui.separator();
            condition(ui, &mut car);
        });
    garage.open = open;
    Ok(())
}

/// The parts to choose from, a row per slot.
fn parts(ui: &mut egui::Ui, garage: &mut Garage) {
    ui.horizontal(|ui| {
        ui.label("Start from:");
        if ui.button("Stock").clicked() {
            garage.draft = Parts::stock();
        }
        if ui.button("Stock turbo").clicked() {
            garage.draft = Parts::turbo();
        }
    });
    let draft = &mut garage.draft;
    egui::Grid::new("parts").num_columns(2).show(ui, |ui| {
        for (i, slot) in SLOTS.iter().enumerate() {
            ui.label(slot.name).on_hover_text(slot.about);
            let enabled = i != INTERCOOLER || draft.has_turbo();
            let chosen = &mut draft.chosen[i];
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt(slot.name)
                    .width(200.0)
                    .selected_text(slot.parts[*chosen].name)
                    .show_ui(ui, |ui| {
                        for (j, part) in slot.parts.iter().enumerate() {
                            ui.selectable_value(chosen, j, part.name);
                        }
                    });
            });
            ui.end_row();
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
            .width(200.0)
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

/// What the parts make, and building it.
fn build(ui: &mut egui::Ui, garage: &mut Garage, car: &mut Car) {
    let hardware = garage.draft.hardware();
    let current = &car.setup.engine.build;
    let rebake = maps::fingerprint(&hardware) != maps::fingerprint(&current.hardware);
    ui.label(format!(
        "{}: {:.0} cm³, {:.1}:1",
        hardware.name,
        hardware.displacement() * 1e6,
        hardware.compression_ratio
    ));
    let changed = garage.draft != car.setup.engine.parts;
    ui.label(if !changed {
        "This is the engine in the car."
    } else if rebake {
        "The cylinder side changes: its maps need baking (a few seconds)."
    } else {
        "Only bolt-on parts change: it fits right away."
    });
    if hardware.has_cam_control() && !garage.draft.ecu.cam_control() {
        ui.colored_label(
            egui::Color32::from_rgb(255, 170, 60),
            "The stock ECU can't drive the switching cam or the phasers: they stay on the \
             mild profile, at rest.",
        );
    }
    ui.checkbox(&mut garage.retune, "Factory tune for the new parts")
        .on_hover_text("Otherwise the ECU keeps its tune, made for the old parts");

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
            if ui
                .add_enabled(changed, egui::Button::new("Build"))
                .clicked()
            {
                start(garage, car, hardware);
            }
            if ui
                .add_enabled(changed, egui::Button::new("Revert"))
                .clicked()
            {
                garage.draft = car.setup.engine.parts.clone();
            }
        });
    }
    if !garage.status.is_empty() {
        ui.label(&garage.status);
    }
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

/// Builds the draft: right away if the maps are at hand (the car's own, the stock ones or
/// cached ones), else starts baking them.
fn start(garage: &mut Garage, car: &mut Car, hardware: EngineHardware) {
    let parts = garage.draft.clone();
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
        fit(car, parts, hardware, maps, garage.retune);
        garage.status = "Built.".to_string();
        return;
    }
    garage.status.clear();
    garage.job = Some(Job {
        car: car.index,
        parts,
        work: Work::start(&hardware),
        hardware,
        retune: garage.retune,
    });
}
