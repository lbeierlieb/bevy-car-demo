//! The garage at home, a scene of its own: a workshop with a bay for mom's car and one for
//! each of the player's, parts on the shelves, and a window to pick the car to take out,
//! repair it and fit parts (see `fitting`; engine parts on the cylinder side need the
//! engine's maps baked, a few seconds, before the car can go out). The room is built deep under home (out of sight of the map),
//! while the player is in it.

use std::f32::consts::FRAC_PI_2;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::fitting::{self, finish_engine};
use super::hud::News;
use super::places::PlaceKind;
use super::{Career, GARAGE_BAYS, OwnedCar, Places, Ride, Scene, Wear, catalog, euros};
use crate::camera::{ChaseCamera, OrbitInput};
use crate::physics::engine::families::Version;
use crate::physics::engine::parts::{Parts, SLOTS};
use crate::physics::tire::COMPOUNDS;
use crate::terrain::Terrain;
use crate::visuals::spawn_model;
use crate::workshop::{self, Work};

/// How far under the ground at home the room is (m).
const DEPTH: f32 = 200.0;
/// The room's half width and half depth, and its height (m); the door is at +z.
const HALF_WIDTH: f32 = 11.5;
const HALF_DEPTH: f32 = 6.5;
const HEIGHT: f32 = 3.6;
/// Bays across the room, where the cars stand (facing the door).
const BAY_WIDTH: f32 = 5.5;
const BAY_Z: f32 = -1.4;
/// The camera: how far from the car, how high, and how far it may swing round (rad).
const VIEW_DISTANCE: f32 = 6.2;
const VIEW_HEIGHT: f32 = 1.5;
const VIEW_SWING: f32 = 1.2;

pub struct GaragePlugin;

impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GarageView>()
            .add_systems(
                OnEnter(Scene::Garage),
                (enter, build_room, show_cars).chain(),
            )
            .add_systems(OnExit(Scene::Garage), leave)
            .add_systems(Update, bake)
            .add_systems(Update, (pick, look).chain().run_if(in_state(Scene::Garage)))
            .add_systems(
                EguiPrimaryContextPass,
                (window, bay_labels).run_if(in_state(Scene::Garage)),
            );
    }
}

/// Everything in the room, which goes when the player leaves.
#[derive(Component)]
struct InGarage;

/// A car shown in a bay (0 is mom's).
#[derive(Component)]
struct BayCar;

/// Where the room is, the bay picked (0 mom's car, then the player's), and the camera's
/// swing round it.
#[derive(Resource, Default)]
struct GarageView {
    origin: Vec3,
    bay: usize,
    swing: f32,
    /// Where the camera is, eased towards where it should be.
    eye: Option<Vec3>,
    status: String,
    /// An engine being put together: its maps baking.
    job: Option<EngineJob>,
}

/// One of the player's cars (by its place) getting an engine of `parts`, which had
/// `before` when the job started.
struct EngineJob {
    car: usize,
    before: Parts,
    parts: Parts,
    work: Work,
}

impl GarageView {
    /// The middle of bay `bay` on the floor.
    fn bay_at(&self, bay: usize) -> Vec3 {
        let first = -BAY_WIDTH * (GARAGE_BAYS as f32) / 2.0;
        self.origin + Vec3::new(first + BAY_WIDTH * bay as f32, 0.0, BAY_Z)
    }

    /// Whether `ride`'s engine is being put together.
    fn busy(&self, ride: Ride) -> bool {
        matches!((ride, &self.job), (Ride::Own(i), Some(job)) if job.car == i)
    }

    fn ride(&self) -> Ride {
        match self.bay {
            0 => Ride::Mom,
            bay => Ride::Own(bay - 1),
        }
    }
}

fn enter(
    career: Res<Career>,
    places: Res<Places>,
    terrain: Res<Terrain>,
    mut view: ResMut<GarageView>,
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
) {
    let home = places.of(PlaceKind::Home).at;
    let ground = terrain.sample(home.x, home.y).0;
    view.origin = Vec3::new(home.x, ground - DEPTH, home.y);
    view.bay = match career.ride {
        Ride::Mom => 0,
        Ride::Own(i) => i + 1,
    };
    view.swing = 0.0;
    view.eye = None;
    view.status.clear();
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (1.0, 0.05);
        }
    }
}

fn leave(
    mut commands: Commands,
    things: Query<Entity, With<InGarage>>,
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    for entity in &things {
        commands.entity(entity).despawn();
    }
    for (camera, mut projection) in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = camera.lens();
        }
    }
}

/// The room: floor with the bays marked, walls, the door, ceiling lights, a workbench and
/// shelves (with a box on them for every part waiting to be fitted).
fn build_room(
    mut commands: Commands,
    view: Res<GarageView>,
    career: Res<Career>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let o = view.origin;
    let mut material = |color: Color, roughness: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            ..default()
        })
    };
    let concrete = material(Color::srgb(0.42, 0.42, 0.40), 0.9);
    let wall = material(Color::srgb(0.78, 0.76, 0.70), 0.95);
    let paint = material(Color::srgb(0.95, 0.80, 0.15), 0.6);
    let door = material(Color::srgb(0.55, 0.58, 0.62), 0.5);
    let wood = material(Color::srgb(0.50, 0.33, 0.18), 0.8);
    let steel = material(Color::srgb(0.25, 0.27, 0.30), 0.5);
    let boxes = [
        material(Color::srgb(0.70, 0.18, 0.12), 0.7),
        material(Color::srgb(0.15, 0.35, 0.70), 0.7),
        material(Color::srgb(0.75, 0.55, 0.30), 0.9),
    ];
    let lamp = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(8.0, 8.0, 7.0),
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut block = |at: Vec3, size: Vec3, material: &Handle<StandardMaterial>| {
        commands.spawn((
            InGarage,
            Mesh3d(cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(o + at).with_scale(size),
        ));
    };
    let (w, d, h) = (HALF_WIDTH, HALF_DEPTH, HEIGHT);
    let mut lights = Vec::new();
    // Floor, ceiling, and the walls (the front one the door).
    block(
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * d),
        &concrete,
    );
    block(
        Vec3::new(0.0, h + 0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * d),
        &wall,
    );
    block(
        Vec3::new(0.0, h / 2.0, -d - 0.1),
        Vec3::new(2.0 * w, h, 0.2),
        &wall,
    );
    block(
        Vec3::new(-w - 0.1, h / 2.0, 0.0),
        Vec3::new(0.2, h, 2.0 * d),
        &wall,
    );
    block(
        Vec3::new(w + 0.1, h / 2.0, 0.0),
        Vec3::new(0.2, h, 2.0 * d),
        &wall,
    );
    block(
        Vec3::new(0.0, h / 2.0, d + 0.1),
        Vec3::new(2.0 * w, h, 0.2),
        &wall,
    );
    // A roller door in front of each bay: slats.
    for bay in 0..=GARAGE_BAYS {
        let x = view.bay_at(bay).x - o.x;
        for k in 0..12 {
            let y = 0.15 + k as f32 * 0.24;
            block(Vec3::new(x, y, d - 0.02), Vec3::new(4.6, 0.2, 0.06), &door);
        }
        // The bay's lines on the floor, and a lamp above it.
        for side in [-1.0, 1.0] {
            let at = Vec3::new(x + side * BAY_WIDTH / 2.0, 0.005, 0.0);
            block(at, Vec3::new(0.08, 0.01, 2.0 * d - 1.0), &paint);
        }
        block(
            Vec3::new(x, h - 0.05, BAY_Z),
            Vec3::new(0.3, 0.06, 2.4),
            &lamp,
        );
        lights.push(o + Vec3::new(x, h - 0.4, BAY_Z + 1.2));
    }
    // The workbench along the left wall, with tools on a board above it.
    block(
        Vec3::new(-w + 0.45, 0.45, 2.0),
        Vec3::new(0.9, 0.9, 3.6),
        &wood,
    );
    block(
        Vec3::new(-w + 0.05, 1.6, 2.0),
        Vec3::new(0.05, 1.0, 3.4),
        &steel,
    );
    for k in 0..6 {
        let z = 0.7 + k as f32 * 0.55;
        block(
            Vec3::new(-w + 0.1, 1.6, z),
            Vec3::new(0.06, 0.5, 0.06),
            &boxes[k % 2],
        );
    }
    // Shelves along the back wall, with the parts waiting to be fitted.
    for level in 0..3 {
        let y = 0.5 + level as f32 * 0.9;
        block(
            Vec3::new(0.0, y, -d + 0.35),
            Vec3::new(2.0 * w - 1.0, 0.04, 0.6),
            &steel,
        );
    }
    for (k, _) in career.items.iter().enumerate().take(60) {
        let (level, place) = (k / 20, k % 20);
        let x = -w + 1.2 + place as f32 * 1.05;
        let y = 0.5 + level as f32 * 0.9 + 0.2;
        block(
            Vec3::new(x, y, -d + 0.35),
            Vec3::new(0.7, 0.36, 0.45),
            &boxes[k % 3],
        );
    }
    for at in lights {
        commands.spawn((
            InGarage,
            PointLight {
                intensity: 600_000.0,
                range: 25.0,
                ..default()
            },
            Transform::from_translation(at),
        ));
    }
}

/// A model of each car in its bay: mom's in the first, the player's after it.
fn show_cars(
    mut commands: Commands,
    view: Res<GarageView>,
    career: Res<Career>,
    shown: Query<Entity, With<BayCar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for entity in &shown {
        commands.entity(entity).despawn();
    }
    let setups = std::iter::once(&career.mom).chain(career.cars.iter().map(|car| &car.setup));
    for (bay, setup) in setups.enumerate() {
        // Facing the door: the model's forward (-z) turned round to +z.
        let root = commands
            .spawn((
                InGarage,
                BayCar,
                Transform::from_translation(view.bay_at(bay))
                    .with_rotation(Quat::from_rotation_y(2.0 * FRAC_PI_2)),
                Visibility::default(),
            ))
            .id();
        spawn_model(
            &mut commands,
            root,
            &setup.build,
            &mut meshes,
            &mut materials,
        );
    }
}

/// Left and right (A and D, or the arrows) pick a bay; Enter drives out.
fn pick(
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<GarageView>,
    mut career: ResMut<Career>,
    mut scene: ResMut<NextState<Scene>>,
) {
    let bays = career.cars.len() + 1;
    let left = keys.any_just_pressed([KeyCode::ArrowLeft, KeyCode::KeyA]);
    let right = keys.any_just_pressed([KeyCode::ArrowRight, KeyCode::KeyD]);
    if left {
        view.bay = (view.bay + bays - 1) % bays;
    }
    if right {
        view.bay = (view.bay + 1) % bays;
    }
    if left || right {
        view.swing = 0.0;
    }
    if keys.just_pressed(KeyCode::Enter) {
        drive_out(&view, &mut career, &mut scene);
    }
}

/// Out in the picked car, unless its engine is still in pieces.
fn drive_out(view: &GarageView, career: &mut Career, scene: &mut NextState<Scene>) {
    if view.busy(view.ride()) {
        return;
    }
    career.ride = view.ride();
    career.save();
    scene.set(Scene::Driving);
}

/// Moves the engine job on; when the maps are baked, the engine goes into the car.
fn bake(mut view: ResMut<GarageView>, mut career: ResMut<Career>, mut news: ResMut<News>) {
    let Some(job) = &mut view.job else {
        return;
    };
    let Some(baked) = job.work.poll() else {
        return;
    };
    let job = view.job.take().expect("a job");
    let Some(maps) = baked else {
        news.say("Putting the engine together failed.");
        return;
    };
    workshop::keep(&job.parts.hardware(), &maps);
    match career.cars.get_mut(job.car) {
        Some(car) if car.setup.engine.parts == job.before => {
            finish_engine(car, job.parts, maps);
            news.say(format!("The {}'s engine is back together.", car.setup.name));
            career.save();
        }
        _ => {}
    }
}

/// The camera on the picked car, from in front and a little to the side; dragging swings
/// it round.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    orbit: Res<OrbitInput>,
    mut view: ResMut<GarageView>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if orbit.0 && buttons.pressed(MouseButton::Left) {
        view.swing = (view.swing - motion.delta.x * 0.005).clamp(-VIEW_SWING, VIEW_SWING);
    }
    let target = view.bay_at(view.bay) + Vec3::new(0.0, 0.7, 0.0);
    let angle = 0.45 + view.swing;
    let mut eye = target + Vec3::new(angle.sin(), 0.0, angle.cos()) * VIEW_DISTANCE;
    eye.y = view.origin.y + VIEW_HEIGHT;
    let inside = Vec3::new(HALF_WIDTH - 0.4, 0.0, HALF_DEPTH - 0.4);
    let o = view.origin;
    eye.x = eye.x.clamp(o.x - inside.x, o.x + inside.x);
    eye.z = eye.z.clamp(o.z - inside.z, o.z + inside.z);
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = view.eye.map_or(eye, |was| was.lerp(eye, ease));
    view.eye = Some(now);
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(target, Vec3::Y);
    }
}

/// The bays: pick a car, see its state, repair it, drive out.
fn window(
    mut contexts: EguiContexts,
    mut view: ResMut<GarageView>,
    mut career: ResMut<Career>,
    mut news: ResMut<News>,
    mut scene: ResMut<NextState<Scene>>,
) -> Result {
    let mut out = false;
    egui::Window::new("Garage")
        .anchor(Align2::LEFT_TOP, egui::vec2(12.0, 120.0))
        .collapsible(false)
        .resizable(false)
        .default_width(330.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(RichText::new("Pick a car (A/D), Enter drives out.").weak());
            let bays = career.cars.len() + 1;
            for bay in 0..bays {
                let name = if bay == 0 {
                    format!("Mom's {}", career.mom.name)
                } else {
                    let car = &career.cars[bay - 1];
                    format!(
                        "{}  ·  {}",
                        car.setup.name, car.setup.engine.build.hardware.name
                    )
                };
                if ui.selectable_label(view.bay == bay, name).clicked() {
                    view.bay = bay;
                    view.swing = 0.0;
                }
            }
            for _ in bays..=GARAGE_BAYS {
                ui.label(RichText::new("(empty bay)").weak());
            }
            ui.separator();
            match view.ride() {
                Ride::Mom => {
                    ui.label("Mom keeps it full and in one piece. Don't tell her about the kerbs.");
                }
                Ride::Own(i) => {
                    let status = condition(ui, &mut career, i, &mut news);
                    if let Some(status) = status {
                        view.status = status;
                    }
                    ui.separator();
                    shelf(ui, &mut career, i, &mut view, &mut news);
                }
            }
            if career.cars.is_empty() && career.pizzas + career.deliveries == 0 {
                ui.separator();
                ui.label(
                    "Mom says you can borrow her car. Pizzeria Bella Strada in town needs \
                     drivers, and Kessler Cars has cars to deliver once they trust you.",
                );
            }
            if !view.status.is_empty() {
                ui.label(&view.status);
            }
            ui.separator();
            let busy = view.busy(view.ride());
            out = ui
                .add_enabled(
                    !busy,
                    egui::Button::new(RichText::new("Drive out  (Enter)").strong()),
                )
                .clicked();
        });
    if out {
        drive_out(&view, &mut career, &mut scene);
    }
    Ok(())
}

/// One of the player's cars: its fuel, tires and engine, and repairing them. Gives a
/// status line when something was done.
fn condition(ui: &mut egui::Ui, career: &mut Career, i: usize, news: &mut News) -> Option<String> {
    let money = career.money;
    let OwnedCar { setup, wear, .. } = &mut career.cars[i];
    let cylinders = setup.engine.build.hardware.cylinders as usize;
    ui.label(format!(
        "{:.0} of {:.0} L in the tank",
        wear.fuel, setup.tank.litres
    ));
    let worn = wear.treads.iter().map(|t| t.wear).fold(0.0, f32::max);
    ui.label(format!(
        "{} tires, {:.0} % worn",
        COMPOUNDS[setup.build.tires].name,
        worn * 100.0
    ));
    let damage = &wear.damage;
    let failures = damage.failures(cylinders);
    if damage.worst(cylinders) < 0.005 {
        ui.label("The engine is in good shape.");
    } else {
        ui.label(format!(
            "Engine wear: {:.0} % at worst",
            damage.worst(cylinders) * 100.0
        ));
    }
    for failure in &failures {
        ui.colored_label(Color32::from_rgb(255, 90, 70), failure);
    }
    let mut done = None;
    let repair = catalog::repair(damage, cylinders);
    if repair > 0.0 {
        let button = egui::Button::new(format!("Repair the engine: {}", euros(repair)));
        if ui.add_enabled(money >= repair, button).clicked() {
            wear.damage = Default::default();
            done = Some((repair, "The engine is repaired.".to_string()));
        }
    }
    let tires = catalog::TIRES[setup.build.tires];
    if worn > 0.05 {
        let button = egui::Button::new(format!("New tires: {}", euros(tires)));
        if ui.add_enabled(money >= tires, button).clicked() {
            wear.treads = Wear::new(setup).treads;
            done = Some((tires, "New tires are on.".to_string()));
        }
    }
    let (price, status) = done?;
    career.money -= price;
    career.save();
    news.say(format!("{status} ({})", euros(price)));
    Some(status)
}

/// Which bay is whose, above the cars.
fn bay_labels(
    mut contexts: EguiContexts,
    view: Res<GarageView>,
    career: Res<Career>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let Ok((camera, eye)) = cameras.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    for bay in 0..=GARAGE_BAYS {
        let text = match bay {
            0 => "Mom's".to_string(),
            b if b <= career.cars.len() => format!("Bay {b}"),
            b => format!("Bay {b} (empty)"),
        };
        let top = view.bay_at(bay) + Vec3::new(0.0, 2.2, 0.0);
        let Ok(at) = camera.world_to_viewport(eye, top) else {
            continue;
        };
        let color = if bay == view.bay {
            Color32::from_rgb(255, 200, 60)
        } else {
            Color32::WHITE
        };
        let galley = painter.layout_no_wrap(text, FontId::proportional(15.0), color);
        let rect = Align2::CENTER_BOTTOM
            .anchor_size(egui::pos2(at.x, at.y), galley.size())
            .expand(4.0);
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(160));
        painter.galley(rect.min + egui::vec2(4.0, 4.0), galley, color);
    }
    Ok(())
}

/// What's fitted that the factory didn't, and the parts on the shelf to fit.
fn shelf(ui: &mut egui::Ui, career: &mut Career, i: usize, view: &mut GarageView, news: &mut News) {
    let car = &career.cars[i];
    let build = &car.setup.build;
    let parts = &car.setup.engine.parts;
    let factory = Version::parts(parts.family, parts.version);
    let mut fitted: Vec<String> = (0..SLOTS.len())
        .filter(|&slot| parts.chosen[slot] != factory.chosen[slot])
        .map(|slot| {
            fitting::name(super::Item::Engine {
                slot,
                part: parts.chosen[slot],
            })
        })
        .collect();
    let stock = crate::chassis::CarBuild::factory(build.chassis);
    for (differs, item) in [
        (build.tires != stock.tires, super::Item::Tires(build.tires)),
        (
            build.clutch != stock.clutch,
            super::Item::Clutch(build.clutch),
        ),
        (
            build.suspension != stock.suspension,
            super::Item::Suspension(build.suspension),
        ),
        (
            build.gearbox != stock.gearbox,
            super::Item::Gearbox(build.gearbox),
        ),
        (
            build.limited_slip && !stock.limited_slip,
            super::Item::LimitedSlip,
        ),
        (parts.ecu != factory.ecu, super::Item::Ecu(parts.ecu)),
    ] {
        if differs {
            fitted.push(fitting::name(item));
        }
    }
    if fitted.is_empty() {
        ui.label("As it left the factory.");
    } else {
        ui.label(format!("Fitted: {}", fitted.join(", ")));
    }
    if let Some(job) = view.job.as_ref().filter(|job| job.car == i) {
        ui.add(
            egui::ProgressBar::new(job.work.progress())
                .desired_width(250.0)
                .text("Putting the engine together…"),
        );
        return;
    }
    if career.items.is_empty() {
        ui.label(RichText::new("No parts on the shelf: order them at Motor Parts Thiel.").weak());
        return;
    }
    ui.strong("On the shelf");
    let mut fit = None;
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            for (k, &item) in career.items.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("Fit").clicked() {
                        fit = Some(k);
                    }
                    ui.label(fitting::name(item));
                });
            }
        });
    ui.label(
        RichText::new(
            "The ECU keeps its tune when parts change: the engine may run rich or lean \
             until it's tuned for them.",
        )
        .weak()
        .small(),
    );
    let Some(k) = fit else {
        return;
    };
    let item = career.items[k];
    match fitting::fit(&mut career.cars[i], item) {
        Err(why) => view.status = why,
        Ok(fitted) => {
            career.items.remove(k);
            career.items.extend(fitted.off);
            view.status = format!("{} fitted.", fitting::name(item));
            if let Some(parts) = fitted.engine {
                let car = &mut career.cars[i];
                let hardware = parts.hardware();
                match workshop::at_hand(&hardware) {
                    Some(maps) => finish_engine(car, parts, maps),
                    None => {
                        view.job = Some(EngineJob {
                            car: i,
                            before: car.setup.engine.parts.clone(),
                            parts,
                            work: Work::start(&hardware),
                        });
                        view.status = "The engine is apart: putting it together…".to_string();
                    }
                }
            }
            news.say(view.status.clone());
            // With the engine apart, the career is saved once it's back together (until
            // then, the save still has the part on the shelf).
            if view.job.is_none() {
                career.save();
            }
        }
    }
}
