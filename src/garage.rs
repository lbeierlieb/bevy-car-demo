//! The showroom (6) and the garage (4). The showroom has every car as it leaves the
//! factory; the garage builds the active car from parts: what's fitted to the chassis
//! (paint, gearbox, clutch, final drive, differentials, suspension, rims and tires on
//! each axle), which engine (any that fits the engine bay), and the engine's own parts.
//!
//! Parts on the cylinder side need the engine's maps baked again (see `workshop`), which
//! takes a few seconds, while the car keeps its old engine.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::car::{ActiveCar, Car, CarSetup, heading};
use crate::chassis::{
    BRAKE_BALANCES, BRAKE_FORCES, CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, PRESSURES,
    SUSPENSIONS, TOES, needs_adapter,
};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::families::{FAMILIES, Version};
use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, EngineMaps};
use crate::physics::engine::parts::{HEADERS, INTERCOOLER, Parts, SLOTS};
use crate::physics::tire::COMPOUNDS;
use crate::shops::{AtShop, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;
use crate::ui::{ToolInput, tool_window};
use crate::wheels::{self, TireSize, Wheel};
use crate::workshop::{self, Work};

const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 170, 60);

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Garage>()
            .add_systems(Update, bake)
            .add_systems(EguiPrimaryContextPass, (window, showroom));
    }
}

#[derive(Resource)]
struct Garage {
    /// Whether the garage was expanded last frame.
    expanded: bool,
    /// The showroom: the chassis looked at, and the engine option and paint picked for it.
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
    paint: usize,
}

impl Default for Garage {
    fn default() -> Self {
        let stock = CarSetup::default();
        Self {
            expanded: false,
            showroom: Showroom {
                chassis: stock.build.chassis,
                engine: 0,
                paint: stock.build.paint,
            },
            car: None,
            same_wheels: stock.build.wheels[0] == stock.build.wheels[1],
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
    workshop::keep(&job.hardware, &maps);
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
    input: ToolInput,
    terrain: Res<Terrain>,
    active_car: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    mut garage: ResMut<Garage>,
) -> Result {
    let Some((entity, mut car)) = active(&mut cars, active_car.0) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let tool = ("Garage", KeyCode::Digit4);
    let Some((window, opened)) = tool_window(ctx, tool, 1, &input, &mut garage.expanded) else {
        return Ok(());
    };
    if opened {
        // Take the parts from the car again when it opens.
        garage.car = None;
    }
    if garage.car != Some(car.index) || garage.draft.0.chassis != car.setup.build.chassis {
        garage.car = Some(car.index);
        garage.draft = (car.setup.build.clone(), car.setup.engine.parts.clone());
        garage.same_wheels = car.setup.build.wheels[0] == car.setup.build.wheels[1];
    }
    let mut build_now = false;
    window.default_height(680.0).vscroll(true).show(ctx, |ui| {
        let garage = &mut *garage;
        car_parts(ui, &mut garage.draft.0);
        wheel_parts(ui, &mut garage.draft.0, &mut garage.same_wheels);
        ui.separator();
        engine(ui, garage);
        build_now = build(ui, garage, &car);
        condition(ui, &mut car);
        ui.separator();
        egui::CollapsingHeader::new("Engine parts")
            .default_open(true)
            .show(ui, |ui| parts(ui, garage));
    });
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
        ui.label("Another car: at a car dealer");
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
        ui.label("Suspension").on_hover_text(
            "Stiffer springs and dampers, aligned to more negative camber, and more room to set \
             it",
        );
        let mut kit = draft.suspension;
        combo(ui, "suspension", &mut kit, &SUSPENSIONS.map(|kit| kit.0));
        if kit != draft.suspension {
            draft.set_suspension(kit);
        }
        ui.end_row();
        for (axle, place) in ["Front", "Rear"].into_iter().enumerate() {
            ui.label(format!("{place} anti-roll bar")).on_hover_text(
                "A stiffer bar rolls the body less and moves more of the load across this \
                 axle in a corner: stiffer at the front makes the car push wider, at the \
                 rear makes it turn in more and step out sooner",
            );
            let bars = &chassis.bars[axle];
            let factory = |bar: usize| {
                let label = bars.label(bar);
                if bar == bars.factory {
                    format!("{label}, factory")
                } else {
                    label
                }
            };
            egui::ComboBox::from_id_salt(("bar", axle))
                .width(220.0)
                .selected_text(factory(draft.bars[axle]))
                .show_ui(ui, |ui| {
                    for bar in 0..bars.sizes.len() {
                        ui.selectable_value(&mut draft.bars[axle], bar, factory(bar));
                    }
                });
            ui.end_row();
        }
    });
}

/// The rims and tires on each axle, or the same all round.
fn wheel_parts(ui: &mut egui::Ui, draft: &mut CarBuild, same: &mut bool) {
    ui.horizontal(|ui| {
        ui.strong("Wheels");
        ui.checkbox(same, "Same front and rear");
    });
    if *same {
        draft.wheels[1] = draft.wheels[0];
    }
    let axles: &[usize] = if *same { &[0] } else { &[0, 1] };
    egui::Grid::new("wheels").num_columns(2).show(ui, |ui| {
        for &axle in axles {
            let wheel = &mut draft.wheels[axle];
            ui.label(if *same {
                "All round"
            } else {
                ["Front", "Rear"][axle]
            });
            wheel_picker(ui, axle, wheel);
            ui.end_row();
            ui.label("");
            ui.label(format!(
                "{}: {:.0} mm across, {:.1} kg a wheel",
                wheel.tire.label(),
                2000.0 * wheel.tire.radius(),
                wheel.mass()
            ))
            .on_hover_text(
                "Wider tires grip more but weigh more and roll heavier; bigger wheels spin up \
                 slower, and taller tires gear the car longer",
            );
            ui.end_row();
        }
    });
    if *same {
        draft.wheels[1] = draft.wheels[0];
    }
    if let Some(why) = draft.wheel_misfit() {
        ui.colored_label(WARNING, why);
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
    ui.strong("Alignment and pressures").on_hover_text(
        "Negative camber leans the tops of the wheels in: it evens out the load across the \
         outside tire in a corner, but wears and heats the inside shoulders on the straight. \
         Tires warm up and gain pressure as they work: too little and they carry on their \
         shoulders, too much and on their middle. Read the tire temperatures across the tread \
         (2) to set both. Toe-in steadies an axle, toe-out makes it keener to turn.",
    );
    egui::Grid::new("alignment").num_columns(5).show(ui, |ui| {
        for axle in 0..2 {
            ui.label(["Front", "Rear"][axle]);
            let range = build.camber_range(axle);
            ui.add(
                egui::Slider::new(&mut build.camber[axle], range)
                    .step_by(0.1)
                    .fixed_decimals(1)
                    .suffix("° camber"),
            );
            ui.add(
                egui::Slider::new(&mut build.toe[axle], TOES)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .suffix("° toe-in"),
            );
            ui.add(
                egui::Slider::new(&mut build.pressure[axle], PRESSURES)
                    .step_by(0.05)
                    .fixed_decimals(2)
                    .suffix(" bar"),
            );
            let wheel = &build.wheels[axle];
            let suited = wheel
                .tire
                .params(wheel.compound)
                .suited_pressure(loads[axle]);
            ui.label(format!("suits {suited:.1}"))
                .on_hover_text("Cold, for the tires to carry evenly once warm, standing");
            ui.end_row();
        }
    });
    ui.strong("Brakes");
    let factory_torque = setup.brake_torque / setup.build.brake_force;
    let factory_bias = setup.brake_bias - setup.build.brake_balance;
    egui::Grid::new("brakes").num_columns(2).show(ui, |ui| {
        ui.label("Force").on_hover_text(
            "Stronger brakes stop shorter until the tires lock (ABS holds them back); the \
             factory's lock a dry road's tires at about 0.8 of the pedal",
        );
        let mut percent = build.brake_force * 100.0;
        let torque = factory_torque * build.brake_force;
        ui.add(
            egui::Slider::new(
                &mut percent,
                BRAKE_FORCES.start() * 100.0..=BRAKE_FORCES.end() * 100.0,
            )
            .step_by(1.0)
            .fixed_decimals(0)
            .suffix(format!(" % of the factory's ({torque:.0} N·m)")),
        );
        build.brake_force = percent / 100.0;
        ui.end_row();
        ui.label("Balance").on_hover_text(
            "More at the front keeps the rear wheels from locking first (stable); more at \
             the rear stops harder until the rear steps out",
        );
        let mut front = (factory_bias + build.brake_balance) * 100.0;
        ui.add(
            egui::Slider::new(
                &mut front,
                (factory_bias + BRAKE_BALANCES.start()) * 100.0
                    ..=(factory_bias + BRAKE_BALANCES.end()) * 100.0,
            )
            .step_by(1.0)
            .fixed_decimals(0)
            .suffix(format!(" % front (factory {:.0} %)", factory_bias * 100.0)),
        );
        build.brake_balance = front / 100.0 - factory_bias;
        ui.end_row();
    });
    build.clamp_alignment();
}

/// Rim diameter and width, tire width and sidewall, and compound. Another rim diameter or
/// tire width keeps the outside diameter as near as it goes, on a rim that suits the tire.
fn wheel_picker(ui: &mut egui::Ui, axle: usize, wheel: &mut Wheel) {
    fn pick<T: Copy + PartialEq>(
        ui: &mut egui::Ui,
        id: (&str, usize),
        chosen: &mut T,
        options: impl Iterator<Item = T>,
        label: impl Fn(T) -> String,
    ) -> bool {
        let before = *chosen;
        egui::ComboBox::from_id_salt(id)
            .width(52.0)
            .selected_text(label(*chosen))
            .show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(chosen, option, label(option));
                }
            });
        *chosen != before
    }
    let diameter = 2.0 * wheel.tire.radius();
    let resize = |wheel: &mut Wheel| {
        wheel.tire = TireSize::nearest(wheel.tire.width, wheel.rim.diameter, diameter);
        wheel.rim.width = wheel.tire.rim_width_near(wheel.rim.width);
    };
    ui.horizontal(|ui| {
        let rims = wheels::RIM_DIAMETERS;
        if pick(ui, ("rim", axle), &mut wheel.rim.diameter, rims, |d| {
            format!("{d}\"")
        }) {
            resize(wheel);
        }
        ui.label("×");
        let widths = wheels::RIM_WIDTHS.step_by(wheels::RIM_WIDTH_STEP);
        pick(ui, ("rim width", axle), &mut wheel.rim.width, widths, |w| {
            format!("{:.1}\"", f32::from(w) / 10.0)
        });
        let widths = wheels::TIRE_WIDTHS.step_by(wheels::TIRE_WIDTH_STEP);
        if pick(ui, ("tire", axle), &mut wheel.tire.width, widths, |w| {
            w.to_string()
        }) {
            resize(wheel);
        }
        ui.label("/");
        let aspects = wheels::ASPECTS.step_by(wheels::ASPECT_STEP);
        pick(ui, ("aspect", axle), &mut wheel.tire.aspect, aspects, |a| {
            a.to_string()
        });
        let compounds = 0..COMPOUNDS.len();
        pick(
            ui,
            ("compound", axle),
            &mut wheel.compound,
            compounds,
            |c| COMPOUNDS[c].name.to_string(),
        );
    })
    .response
    .on_hover_text(COMPOUNDS[wheel.compound].about);
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
    } else if rebake && maps::is_stock(&hardware) {
        "A factory engine: it fits right away."
    } else if rebake {
        "The cylinder side changes: its maps need baking (a few seconds), unless it was \
         built before."
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
            let fits = garage.draft.0.wheel_misfit().is_none();
            build_now = ui
                .add_enabled(changed && fits, egui::Button::new("Build"))
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
    if let Some(why) = build.wheel_misfit() {
        garage.status = format!("{why}.");
        return;
    }
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
        work: Work::start(&hardware),
        hardware,
        retune: garage.retune,
    });
}

/// The car dealer: every car as it leaves the factory, to pick one and drive it.
fn showroom(
    mut commands: Commands,
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    active_car: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    mut garage: ResMut<Garage>,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
) -> Result {
    let Some((entity, mut car)) = active(&mut cars, active_car.0) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let mut shown = garage.showroom;
    let mut drive = false;
    shop_window(ctx, &shops, &mut at, ShopKind::Showroom, |ui, came_in| {
        if came_in {
            garage.status.clear();
            // Start at the car being driven.
            shown = Showroom {
                chassis: car.setup.build.chassis,
                engine: 0,
                paint: car.setup.build.paint,
            };
        }
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
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(330.0);
                drive = brochure(ui, &mut shown);
            });
        });
        if !garage.status.is_empty() {
            ui.label(&garage.status);
        }
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
    garage.showroom = shown;
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
        + 2.0 * (chassis.unsprung(0) + chassis.unsprung(1));
    ui.label(format!("{mass:.0} kg with it"));
    let [front, rear] = chassis.wheels.map(|wheel| wheel.label());
    ui.label(if front == rear {
        format!("On {front}")
    } else {
        format!("On {front} at the front, {rear} at the rear")
    });
    ui.label("(A parts shop fits any engine there's room for)");
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
