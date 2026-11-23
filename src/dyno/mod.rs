//! The dyno shops: the car on an all-wheel chassis dyno (see `rollers`) in a room deep
//! under the shop. From the shop's window the car goes onto the rollers; a run warms up
//! through the gears and pulls in fourth while a screen shows the power at the wheels;
//! then a window shows the power and torque at the wheels and at the crank, and the runs
//! of this visit, each with a name and notes to give it.
//!
//! The car on the rollers is a copy of the driven car, which stays parked on the shop's
//! marker (so other players see it there, a ghost): what the runs do to the engine and
//! the fuel goes back to the driven car when it leaves the room.

mod chart;
mod rollers;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Stroke};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::audio::EngineAudio;
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car, CarSetup};
use crate::ecu_ui::{self, EcuPanel};
use crate::looks::{Looks, Motion};
use crate::menu::kit::{self, Panel, Place};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, EngineMaps};
use crate::physics::engine::parts::Parts;
use crate::shops::{AtShop, SceneBack, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;
use crate::workshop::{self, Work};
use chart::{Axis, HORSEPOWER, Line, chart};
use rollers::{Hold, Rollers, STEP, Sample, Stage};

/// A run needs this much fuel in the tank (L).
const RUN_FUEL: f32 = 1.0;
/// How far under the ground at the shop the room is (m).
const DEPTH: f32 = 200.0;
/// The room's half width (across the car) and half length, and its height (m).
const HALF_WIDTH: f32 = 7.0;
const HALF_LENGTH: f32 = 8.0;
const HEIGHT: f32 = 4.5;
/// The rollers: radius, length, and how far their tops stand out of the floor (m).
const ROLLER_RADIUS: f32 = 0.5;
const ROLLER_LENGTH: f32 = 2.6;
const ROLLER_TOP: f32 = 0.03;
/// The camera's place relative to the car (in front and to its left), and how far it may
/// swing round (rad).
const EYE: Vec3 = Vec3::new(-3.6, 1.45, -4.9);
const VIEW_SWING: f32 = 1.0;
/// How far right of the car the camera looks while the results are up (m).
const RESULTS_ASIDE: f32 = 1.0;
/// The screen: how far in front of the camera, left of the middle, above it, and its size
/// (m).
const SCREEN_AHEAD: f32 = 3.4;
const SCREEN_LEFT: f32 = 1.2;
const SCREEN_UP: f32 = 0.3;
const SCREEN_SIZE: Vec2 = Vec2::new(1.5, 0.9);
/// The runs' colours: power, torque, and a run compared with.
const POWER: Color32 = Color32::from_rgb(255, 170, 50);
const TORQUE: Color32 = Color32::from_rgb(90, 170, 255);
const COMPARED: Color32 = Color32::from_rgb(170, 170, 180);
const SCREEN_INK: Color32 = Color32::from_rgb(120, 255, 160);

pub struct DynoPlugin;

impl Plugin for DynoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DynoView>()
            .add_systems(
                Update,
                (
                    (run, bake),
                    (animate, look.after(crate::camera::follow)),
                    leave_when_gone,
                )
                    .chain()
                    .run_if(in_room),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (shop, (screen, skip, results).run_if(in_room)),
            );
    }
}

/// A run: its number, the name and notes given to it, the car as it was (name and
/// engine), the gear it was pulled in, its samples, and what broke.
#[derive(Clone, Debug, PartialEq)]
struct DynoRun {
    number: u32,
    name: String,
    notes: String,
    car: String,
    gear: i8,
    samples: Vec<Sample>,
    broke: Vec<String>,
}

impl DynoRun {
    /// The highest of `value` over the run, and its rpm.
    fn peak(&self, value: fn(&Sample) -> f32) -> (f32, f32) {
        (self.samples.iter())
            .map(|sample| (sample.rpm, value(sample)))
            .fold(
                (0.0, 0.0),
                |best, now| if now.1 > best.1 { now } else { best },
            )
    }

    fn line(&self, value: fn(&Sample) -> f32) -> Vec<(f32, f32)> {
        (self.samples.iter())
            .map(|sample| (sample.rpm, value(sample)))
            .collect()
    }
}

/// Everything in the room, which goes when the car leaves it.
#[derive(Component)]
struct InDyno;

/// A roller, turning with the wheels on it; and the car on the rollers (drawn and heard
/// like any car from its `Looks` and `Motion`).
#[derive(Component)]
struct Roller;
#[derive(Component)]
struct CarModel;

/// The car on the rollers, not a roller.
type ModelOnly = (With<CarModel>, Without<Roller>);

/// The room and the car in it: where it is, the car on the rollers and its run, the
/// camera, the runs, the run shown in the results and the one compared with.
#[derive(Resource, Default)]
struct DynoView {
    origin: Vec3,
    /// The copy of the driven car on the rollers, while the car is in the room.
    car: Option<Car>,
    rollers: Option<Rollers>,
    /// Time not yet stepped (s), and whether the finished run is filed.
    owed: f32,
    filed: bool,
    swing: f32,
    eye: Option<Vec3>,
    /// How far the view has moved over for the results (0..1).
    aside: f32,
    screen: Transform,
    runs: Vec<DynoRun>,
    shown: Option<usize>,
    compared: Option<usize>,
    /// The cam timing and wastegate being set (the parts they go into), and the maps
    /// baking for them (with the parts and hardware), and what applying did.
    draft: Option<Parts>,
    work: Option<(Parts, EngineHardware, Work)>,
    status: String,
    /// What a hold is set to.
    hold: Hold,
    /// The window's tab, and the ECU tuning's state.
    tab: Tab,
    ecu: EcuPanel,
    /// Something to say about the last run (what broke, or that it measured nothing).
    news: String,
}

/// What the dyno's window shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Runs,
    Ecu,
    Cams,
}

impl DynoView {
    fn running(&self) -> bool {
        self.rollers
            .as_ref()
            .is_some_and(|rollers| rollers.stage != Stage::Done)
    }
}

fn in_room(view: Res<DynoView>) -> bool {
    view.car.is_some()
}

/// The dyno shop's window (outside the room): onto the rollers.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut view: ResMut<DynoView>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if view.car.is_some() {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Dyno, |ui, _| {
        enter = kit::button(ui, "Onto the rollers").clicked();
        kit::note(
            ui,
            "An all-wheel chassis dyno: a run warms up through the gears and pulls in fourth \
             to the rev limiter, measuring the power at the wheels.",
        );
        if !view.runs.is_empty() {
            let runs = view.runs.len();
            kit::note(
                ui,
                &format!("{runs} run{} on file.", if runs == 1 { "" } else { "s" }),
            );
        }
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    view.origin = Vec3::new(marker.x, ground - DEPTH, marker.y);
    view.car = Some(car.clone());
    view.rollers = None;
    view.owed = 0.0;
    view.filed = false;
    view.swing = 0.0;
    view.eye = None;
    view.aside = 0.0;
    view.shown = view.runs.len().checked_sub(1);
    view.compared = None;
    view.news.clear();
    let eye = view.origin + EYE;
    let target = view.origin + Vec3::Y * 0.7;
    let ahead = (target - eye).normalize();
    let right = ahead.cross(Vec3::Y).normalize();
    let at = eye + ahead * SCREEN_AHEAD - right * SCREEN_LEFT + Vec3::Y * SCREEN_UP;
    view.screen = Transform::from_translation(at).looking_at(eye.with_y(at.y), Vec3::Y);
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (1.0, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    build_room(&mut commands, &view, &mut meshes, &mut materials);
    Ok(())
}

/// Off the rollers and back to the driven car, parked at the shop: what the runs did to
/// the engine (wear, heat) and the fuel goes with it, and the tune.
#[allow(clippy::too_many_arguments)]
fn leave(
    commands: &mut Commands,
    view: &mut DynoView,
    things: &Query<Entity, With<InDyno>>,
    cars: &mut Query<&mut Car>,
    active: &ActiveCar,
    away: &mut Away,
    audio: &mut EngineAudio,
    cameras: &mut Query<(&ChaseCamera, &mut Projection)>,
) {
    for entity in things {
        commands.entity(entity).despawn();
    }
    for (camera, mut projection) in cameras.iter_mut() {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = camera.lens();
        }
    }
    view.rollers = None;
    if let Some(copy) = view.car.take()
        && let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0)
    {
        // The car as the room left it (the tune, cam timing, wastegate), as it ran.
        if car.setup.engine.parts != copy.setup.engine.parts {
            let rebuilt = car.rebuilt(copy.setup.clone());
            *car = rebuilt;
        } else {
            car.setup.engine.build.ecu = copy.setup.engine.build.ecu.clone();
        }
        car.drivetrain.engine = copy.drivetrain.engine;
        car.fuel = copy.fuel;
    }
    if let Some((.., work)) = view.work.take() {
        work.cancel();
    }
    view.draft = None;
    away.0 = false;
    audio.hushed = false;
}

/// The car changed (or left the server, and with it the shop) while in the room: out.
#[allow(clippy::too_many_arguments)]
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut view: ResMut<DynoView>,
    things: Query<Entity, With<InDyno>>,
    mut cars: Query<&mut Car>,
    active: Res<ActiveCar>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if at.shop.is_none() {
        leave(
            &mut commands,
            &mut view,
            &things,
            &mut cars,
            &active,
            &mut away,
            &mut audio,
            &mut cameras,
        );
    }
}

/// The room: a concrete cell with the rollers in the floor, the car on them strapped down,
/// a fan in front of it, lights, and the screen on a stand.
fn build_room(
    commands: &mut Commands,
    view: &DynoView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let Some(car) = &view.car else {
        return;
    };
    let o = view.origin;
    let mut material = |color: Color, roughness: f32, metallic: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic,
            ..default()
        })
    };
    let concrete = material(Color::srgb(0.36, 0.37, 0.38), 0.85, 0.0);
    let wall = material(Color::srgb(0.20, 0.22, 0.26), 0.9, 0.0);
    let pit = material(Color::srgb(0.03, 0.03, 0.03), 1.0, 0.0);
    let steel = material(Color::srgb(0.62, 0.64, 0.66), 0.35, 0.8);
    let stripe = material(Color::srgb(0.10, 0.10, 0.11), 0.6, 0.3);
    let yellow = material(Color::srgb(0.95, 0.75, 0.10), 0.6, 0.0);
    let strap = material(Color::srgb(0.85, 0.30, 0.05), 0.8, 0.0);
    let dark = material(Color::srgb(0.08, 0.08, 0.09), 0.5, 0.2);
    let glass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.02, 0.04, 0.06),
        emissive: LinearRgba::rgb(0.02, 0.06, 0.08),
        perceptual_roughness: 0.2,
        ..default()
    });
    let lamp = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(8.0, 8.0, 7.5),
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let block = |commands: &mut Commands, at: Vec3, size: Vec3, material: &Handle<_>| {
        commands.spawn((
            InDyno,
            Mesh3d(cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(o + at).with_scale(size),
        ));
    };
    let (w, l, h) = (HALF_WIDTH, HALF_LENGTH, HEIGHT);
    // The axles, in the room's frame (the car's middle at the origin, facing -z).
    let setup = &car.setup;
    let middle = (setup.front_z + setup.rear_z) / 2.0;
    let axles = [setup.front_z - middle, setup.rear_z - middle];
    // The floor, with a slot over each roller, and a pit under them.
    let slot = 0.42;
    let mut edges = vec![-l];
    for z in axles {
        edges.extend([z - slot, z + slot]);
    }
    edges.push(l);
    for pair in edges.chunks(2) {
        let (from, to) = (pair[0], pair[1]);
        let at = Vec3::new(0.0, -0.1, (from + to) / 2.0);
        let size = Vec3::new(2.0 * w, 0.2, to - from);
        block(commands, at, size, &concrete);
    }
    let pit_half = ROLLER_LENGTH / 2.0 + 0.2;
    for z in axles {
        // The floor beside the slot, the pit's floor and walls.
        for side in [-1.0, 1.0] {
            let at = Vec3::new(side * (pit_half + w) / 2.0, -0.1, z);
            let size = Vec3::new(w - pit_half, 0.2, 2.0 * slot);
            block(commands, at, size, &concrete);
            let at = Vec3::new(side * pit_half, -0.6, z);
            block(commands, at, Vec3::new(0.05, 1.2, 2.0 * slot), &pit);
            let at = Vec3::new(0.0, -0.6, z + side * slot);
            block(commands, at, Vec3::new(2.0 * pit_half, 1.2, 0.05), &pit);
        }
        let at = Vec3::new(0.0, -1.2, z);
        block(
            commands,
            at,
            Vec3::new(2.0 * pit_half, 0.1, 2.0 * slot),
            &pit,
        );
        // Yellow edges round the slot.
        for side in [-1.0, 1.0] {
            let at = Vec3::new(0.0, 0.004, z + side * (slot + 0.05));
            block(commands, at, Vec3::new(2.0 * pit_half, 0.01, 0.1), &yellow);
        }
    }
    // Walls and ceiling.
    let at = Vec3::new(0.0, h + 0.1, 0.0);
    block(commands, at, Vec3::new(2.0 * w, 0.2, 2.0 * l), &wall);
    for side in [-1.0, 1.0] {
        let at = Vec3::new(side * (w + 0.1), h / 2.0, 0.0);
        block(commands, at, Vec3::new(0.2, h, 2.0 * l), &wall);
        let at = Vec3::new(0.0, h / 2.0, side * (l + 0.1));
        block(commands, at, Vec3::new(2.0 * w, h, 0.2), &wall);
    }
    // The rollers, turning about their length (x), with stripes to see them turn.
    let drum = meshes.add(Cylinder::new(ROLLER_RADIUS, ROLLER_LENGTH));
    for z in axles {
        let centre = o + Vec3::new(0.0, ROLLER_TOP - ROLLER_RADIUS, z);
        commands
            .spawn((
                InDyno,
                Roller,
                Transform::from_translation(centre),
                Visibility::default(),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(drum.clone()),
                    MeshMaterial3d(steel.clone()),
                    Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ));
                for k in 0..6 {
                    let angle = k as f32 * std::f32::consts::TAU / 6.0;
                    let at = Quat::from_rotation_x(angle) * Vec3::Y * ROLLER_RADIUS;
                    parent.spawn((
                        Mesh3d(cube.clone()),
                        MeshMaterial3d(stripe.clone()),
                        Transform::from_translation(at)
                            .with_rotation(Quat::from_rotation_x(angle))
                            .with_scale(Vec3::new(ROLLER_LENGTH + 0.01, 0.01, 0.08)),
                    ));
                }
            });
    }
    // The car on them, facing the fan: drawn (and heard) from its looks and motion.
    commands.spawn((
        InDyno,
        CarModel,
        Looks::of(setup),
        Motion::of(car),
        model_at(view, car, 0.0, 0.0),
    ));
    // Straps from the car's ends down to the floor.
    let ends = [axles[0] - 0.75, axles[1] + 0.75];
    for (k, z) in ends.into_iter().enumerate() {
        let out = if k == 0 { -1.0 } else { 1.0 };
        for side in [-1.0, 1.0] {
            let top = o + Vec3::new(side * 0.45, 0.42, z);
            let foot = o + Vec3::new(side * 0.9, 0.0, z + out * 1.1);
            let middle = (top + foot) / 2.0;
            let length = top.distance(foot);
            commands.spawn((
                InDyno,
                Mesh3d(cube.clone()),
                MeshMaterial3d(strap.clone()),
                Transform::from_translation(middle)
                    .looking_at(foot, Vec3::Y)
                    .with_scale(Vec3::new(0.05, 0.01, length)),
            ));
            block(commands, foot - o, Vec3::new(0.16, 0.05, 0.16), &dark);
        }
    }
    // The fan in front, blowing at the radiator.
    let fan_z = axles[0] - 2.9;
    let at = Vec3::new(0.0, 0.7, fan_z);
    block(commands, at, Vec3::new(1.5, 1.4, 0.5), &dark);
    commands.spawn((
        InDyno,
        Mesh3d(meshes.add(Cylinder::new(0.6, 0.05))),
        MeshMaterial3d(steel.clone()),
        Transform::from_translation(o + Vec3::new(0.0, 0.75, fan_z + 0.26))
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
    ));
    // The screen on a stand.
    let screen = view.screen;
    commands.spawn((
        InDyno,
        Mesh3d(cube.clone()),
        MeshMaterial3d(glass),
        screen.with_scale(Vec3::new(SCREEN_SIZE.x + 0.08, SCREEN_SIZE.y + 0.08, 0.05)),
    ));
    let foot = screen.translation.with_y(o.y);
    let pole_height = screen.translation.y - o.y;
    commands.spawn((
        InDyno,
        Mesh3d(cube.clone()),
        MeshMaterial3d(dark.clone()),
        Transform::from_translation(foot + Vec3::Y * pole_height / 2.0 - screen.forward() * 0.05)
            .with_scale(Vec3::new(0.08, pole_height, 0.08)),
    ));
    // Lights in the ceiling.
    for x in [-2.5, 2.5] {
        for z in [-3.5, 0.0, 3.5] {
            let at = Vec3::new(x, h - 0.03, z);
            block(commands, at, Vec3::new(0.25, 0.05, 2.2), &lamp);
            commands.spawn((
                InDyno,
                PointLight {
                    intensity: 450_000.0,
                    range: 20.0,
                    ..default()
                },
                Transform::from_translation(o + Vec3::new(x, h - 0.5, z)),
            ));
        }
    }
}

/// Where the car on the rollers is (its centre of mass, as `Looks` are drawn from):
/// standing on them midway between the axles, pitched by `squat` (rad) and lifted by
/// `shake` (m).
fn model_at(view: &DynoView, car: &Car, squat: f32, shake: f32) -> Transform {
    // The body's design frame (on the ground midway between the axles) is at
    // `body_offset` in the chassis frame, whose origin is the centre of mass.
    let ground = view.origin + Vec3::Y * (ROLLER_TOP + shake);
    let rotation = Quat::from_rotation_x(squat);
    Transform::from_translation(ground - rotation * car.setup.body_offset).with_rotation(rotation)
}

/// Runs the dyno in real time (Space skips to the end), and files the run when it's done.
fn run(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<DynoView>) {
    let view = &mut *view;
    let (Some(car), Some(rollers)) = (&mut view.car, &mut view.rollers) else {
        return;
    };
    if keys.just_pressed(KeyCode::Space) {
        rollers.finish(car);
    }
    view.owed = (view.owed + time.delta_secs()).min(0.1);
    while view.owed >= STEP && rollers.stage != Stage::Done {
        view.owed -= STEP;
        rollers.step(car, STEP);
    }
    if rollers.stage != Stage::Done || view.filed {
        return;
    }
    if rollers.hold.is_some() {
        // A hold measures nothing to file.
        view.filed = true;
        view.news = rollers.broke.join(", ");
        return;
    }
    view.filed = true;
    view.news = if rollers.broke.is_empty() {
        String::new()
    } else {
        format!("Bang! {}", rollers.broke.join(", "))
    };
    if rollers.samples.len() < 2 {
        view.news += " The run didn't get far enough to measure anything.";
        return;
    }
    let number = view.runs.iter().map(|run| run.number).max().unwrap_or(0) + 1;
    let setup = &car.setup;
    view.runs.push(DynoRun {
        number,
        name: format!("Run {number}"),
        notes: String::new(),
        car: format!("{} · {}", setup.name, setup.engine.build.hardware.name),
        gear: rollers.gear,
        samples: rollers.samples.clone(),
        broke: rollers.broke.clone(),
    });
    view.shown = Some(view.runs.len() - 1);
}

/// The rollers and wheels turn; the car squats and shakes under power.
fn animate(
    time: Res<Time>,
    view: Res<DynoView>,
    mut rollers: Query<&mut Transform, (With<Roller>, Without<CarModel>)>,
    mut models: Query<(&mut Transform, &mut Motion), ModelOnly>,
) {
    let Some(car) = &view.car else {
        return;
    };
    let turned = view.rollers.as_ref().map_or(0.0, |run| run.turned);
    let tire = car.setup.axle(false).tire.radius;
    for mut transform in &mut rollers {
        transform.rotation = Quat::from_rotation_x(turned * tire / ROLLER_RADIUS);
    }
    let load = car.drivetrain.load;
    let t = time.elapsed_secs();
    let shake = 0.0015 * load * ((t * 61.0).sin() + (t * 97.0).sin());
    for (mut transform, mut motion) in &mut models {
        *transform = model_at(&view, car, 0.006 * load, shake);
        *motion = Motion::of(car);
        for wheel in &mut motion.wheels {
            wheel.spin = turned;
        }
    }
}

/// The camera on the car from in front and to its left, beside the screen; dragging
/// swings it round.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut view: ResMut<DynoView>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        view.swing = (view.swing - motion.delta.x * 0.005).clamp(-VIEW_SWING, VIEW_SWING);
    }
    let eye = view.origin + Quat::from_rotation_y(view.swing) * EYE;
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = view.eye.map_or(eye, |was| was.lerp(eye, ease));
    view.eye = Some(now);
    // With the window up on the right, the car moves over to the left.
    let aside = 1.0;
    view.aside += (aside - view.aside) * ease * 0.5;
    let car = view.origin + Vec3::Y * 0.7;
    let right = (car - now).cross(Vec3::Y).normalize();
    let target = car + right * RESULTS_ASIDE * view.aside;
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(target, Vec3::Y);
    }
}

/// The dyno's screen: the power at the wheels in big numbers, the gear, rpm and speed,
/// and the pull's curve as it's drawn.
fn screen(
    mut contexts: EguiContexts,
    view: Res<DynoView>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let (Ok((camera, eye)), Some(car)) = (cameras.single(), &view.car) else {
        return Ok(());
    };
    // Where the screen's corners are in the window.
    let s = view.screen;
    let half = SCREEN_SIZE / 2.0;
    let ctx = contexts.ctx_mut()?;
    let mut corners = Vec::new();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let at = s.translation + s.right() * x * half.x + s.up() * y * half.y;
        let Ok(p) = camera.world_to_viewport(eye, at) else {
            return Ok(());
        };
        corners.push(crate::menu::to_points(ctx, p));
    }
    let rect = egui::Rect::from_points(&corners);
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.add(egui::Shape::convex_polygon(
        corners,
        Color32::from_rgb(6, 14, 20),
        Stroke::new(1.0, Color32::from_rgb(40, 60, 70)),
    ));
    let h = rect.height();
    let font = |size: f32| FontId::monospace(size * h);
    let ink = SCREEN_INK;
    let dim = ink.gamma_multiply(0.55);
    let left = rect.left() + 0.05 * rect.width();
    painter.text(
        egui::pos2(left, rect.top() + 0.05 * h),
        Align2::LEFT_TOP,
        "AWD DYNO",
        font(0.07),
        dim,
    );
    let run = view.rollers.as_ref();
    let stage = match run.map(|run| run.stage) {
        None => "READY",
        Some(Stage::Idle) => "IDLE",
        Some(Stage::Gears) => "WARM-UP",
        Some(Stage::Settle) => "SHIFT",
        Some(Stage::Pull) => "PULL",
        Some(Stage::Hold) => "HOLD",
        Some(Stage::Coast) => "COAST DOWN",
        Some(Stage::Done) => "DONE",
    };
    painter.text(
        egui::pos2(rect.right() - 0.05 * rect.width(), rect.top() + 0.05 * h),
        Align2::RIGHT_TOP,
        stage,
        font(0.07),
        ink,
    );
    let samples = run.map_or(&[][..], |run| &run.samples[..]);
    let peak = samples.iter().map(Sample::wheel_hp).fold(0.0, f32::max);
    let finished = matches!(run.map(|run| run.stage), Some(Stage::Done | Stage::Coast));
    let power = match run {
        Some(_) if finished => peak,
        Some(run) => run.power.max(0.0) / HORSEPOWER,
        None => 0.0,
    };
    painter.text(
        egui::pos2(left, rect.top() + 0.15 * h),
        Align2::LEFT_TOP,
        format!("{power:>4.0}"),
        font(0.30),
        ink,
    );
    painter.text(
        egui::pos2(left + 0.58 * rect.width(), rect.top() + 0.36 * h),
        Align2::LEFT_BOTTOM,
        if finished { "PEAK WHP" } else { "WHP" },
        font(0.08),
        dim,
    );
    let drivetrain = &car.drivetrain;
    let gear = match drivetrain.selected_gear() {
        0 => "N".to_string(),
        -1 => "R".to_string(),
        g => g.to_string(),
    };
    let speed = run.map_or(0.0, |run| run.speed() * 3.6);
    let rpm = if drivetrain.running {
        drivetrain.rpm
    } else {
        0.0
    };
    painter.text(
        egui::pos2(left, rect.top() + 0.5 * h),
        Align2::LEFT_TOP,
        format!("GEAR {gear}  {rpm:>5.0} RPM  {speed:>3.0} KM/H"),
        font(0.075),
        ink,
    );
    // The pull's curve, wheel power by rpm.
    let graph = egui::Rect::from_min_max(
        egui::pos2(left, rect.top() + 0.64 * h),
        egui::pos2(rect.right() - 0.05 * rect.width(), rect.bottom() - 0.06 * h),
    );
    painter.rect_stroke(graph, 0.0, Stroke::new(1.0, dim), egui::StrokeKind::Inside);
    let top_rpm = car.setup.engine.build.ecu.rev_limit;
    let top_power = (peak * 1.15).max(50.0);
    let points: Vec<egui::Pos2> = (samples.iter())
        .map(|sample| {
            egui::pos2(
                egui::lerp(graph.left()..=graph.right(), sample.rpm / top_rpm),
                egui::lerp(
                    graph.bottom()..=graph.top(),
                    (sample.wheel_hp() / top_power).clamp(0.0, 1.0),
                ),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(2.0, ink)));
    Ok(())
}

/// While the run is on: skipping it.
fn skip(mut contexts: EguiContexts, mut view: ResMut<DynoView>) -> Result {
    if !view.running() || view.rollers.as_ref().is_some_and(|run| run.hold.is_some()) {
        return Ok(());
    }
    let mut skipped = false;
    egui::Area::new(egui::Id::new("dyno skip"))
        .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
        .show(contexts.ctx_mut()?, |ui| {
            skipped = ui.button("Skip to the results  (Space)").clicked();
        });
    if skipped {
        let view = &mut *view;
        if let (Some(car), Some(rollers)) = (&mut view.car, &mut view.rollers) {
            // `run` files it.
            rollers.finish(car);
        }
    }
    Ok(())
}

/// The dyno's window, on the right: the runs (the shown run's chart and peaks, its name
/// and notes, every run of this visit) or the ECU's tuning, running (again) and going
/// back to the car.
#[allow(clippy::too_many_arguments)]
fn results(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut view: ResMut<DynoView>,
    things: Query<Entity, With<InDyno>>,
    mut cars: Query<&mut Car>,
    active: Res<ActiveCar>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    (mut cameras, mut back): (Query<(&ChaseCamera, &mut Projection)>, ResMut<SceneBack>),
) -> Result {
    let mut again = false;
    let (mut hold, mut stop) = (false, false);
    let mut out = back.take();
    let ctx = contexts.ctx_mut()?;
    let width = match view.tab {
        Tab::Ecu => ecu_ui::WIDTH + 32.0,
        Tab::Runs => 640.0,
        Tab::Cams => 460.0,
    };
    Panel::new("dyno", "Dyno")
        .place(Place::Right)
        .width(width)
        .back(Some("Back to the car"))
        .show(ctx, |ui| {
            let view = &mut *view;
            let tabs = [Tab::Runs, Tab::Ecu, Tab::Cams];
            kit::pick(ui, "Page", &mut view.tab, &tabs, |tab| {
                match tab {
                    Tab::Runs => "Runs",
                    Tab::Ecu => "ECU tune",
                    Tab::Cams => "Cams and boost",
                }
                .to_string()
            });
            let running = view.running();
            let holding = view.rollers.as_ref().and_then(|run| run.hold).is_some() && running;
            let why_not = view.car.as_ref().and_then(cannot_run);
            let can = !running && why_not.is_none();
            let label = if view.rollers.is_some() {
                "Run it again: through the gears, a pull in fourth"
            } else {
                "Run it: through the gears, a pull in fourth"
            };
            again = ui
                .add_enabled_ui(can, |ui| kit::button(ui, label))
                .inner
                .clicked();
            if holding {
                stop = kit::button(ui, "Stop holding").clicked();
            } else {
                hold = ui
                    .add_enabled_ui(can, |ui| {
                        kit::button(ui, "Hold at this rpm and throttle, in fourth")
                    })
                    .inner
                    .clicked();
            }
            // Holding at an rpm and throttle, to tune the cells there.
            let limit =
                (view.car.as_ref()).map_or(7000.0, |car| car.setup.engine.build.ecu.rev_limit);
            kit::slider(
                ui,
                "Hold at",
                &mut view.hold.rpm,
                1000.0..=limit,
                100.0,
                |v| format!("{v:.0} rpm"),
            );
            let mut percent = view.hold.throttle * 100.0;
            kit::slider(ui, "Throttle", &mut percent, 0.0..=100.0, 5.0, |v| {
                format!("{v:.0} %")
            });
            view.hold.throttle = percent / 100.0;
            if let Some(why) = why_not {
                kit::warning(ui, why);
            }
            if kit::button(ui, "Back to the car").clicked() {
                out = true;
            }
            match view.tab {
                Tab::Runs => {
                    if !view.news.is_empty() {
                        kit::warning(ui, view.news.trim());
                    }
                    shown_run(ui, view);
                    run_list(ui, view);
                }
                Tab::Ecu => {
                    if let Some(car) = &mut view.car {
                        ecu_ui::tuning(ui, &mut view.ecu, car);
                    }
                }
                Tab::Cams => cams(ui, view),
            }
        });
    let view = &mut *view;
    if let Some(run) = view.rollers.as_mut().filter(|run| run.hold.is_some()) {
        run.hold = Some(view.hold);
    }
    if let (Some(car), Some(run)) = (&mut view.car, &mut view.rollers)
        && stop
    {
        run.stop(car);
    }
    if (again || hold)
        && let Some(car) = &mut view.car
    {
        let mut run = Rollers::new(car);
        run.hold = hold.then_some(view.hold);
        view.rollers = Some(run);
        view.owed = 0.0;
        view.filed = false;
        view.news.clear();
        if again {
            view.tab = Tab::Runs;
        }
    }
    if out {
        leave(
            &mut commands,
            view,
            &things,
            &mut cars,
            &active,
            &mut away,
            &mut audio,
            &mut cameras,
        );
    }
    Ok(())
}

/// Why `car` can't have a run, if it can't.
fn cannot_run(car: &Car) -> Option<&'static str> {
    if car
        .drivetrain
        .engine
        .blown(&car.setup.engine.build.hardware)
    {
        Some("The engine won't run: it needs a repair at a parts shop.")
    } else if car.fuel.litres() < RUN_FUEL {
        Some("There isn't enough fuel in the tank for a run.")
    } else {
        None
    }
}

/// The shown run: power and torque at the wheels (solid) and at the crank (dashed), the
/// peaks, and its name and notes to edit.
fn shown_run(ui: &mut egui::Ui, view: &mut DynoView) {
    let Some(i) = view.shown.filter(|&i| i < view.runs.len()) else {
        ui.label("No runs yet.");
        return;
    };
    let mut lines = Vec::new();
    if let Some(other) = view.compared.and_then(|k| view.runs.get(k)) {
        for (value, axis) in [
            (Sample::wheel_hp as fn(&Sample) -> f32, Axis::Power),
            (Sample::wheel_torque, Axis::Torque),
        ] {
            lines.push(Line {
                points: other.line(value),
                axis,
                color: COMPARED,
                dashed: false,
                name: None,
            });
        }
    }
    let run = &view.runs[i];
    for (value, axis, color, dashed, name) in [
        (
            Sample::wheel_hp as fn(&Sample) -> f32,
            Axis::Power,
            POWER,
            false,
            "wheels ",
        ),
        (Sample::crank_hp, Axis::Power, POWER, true, "crank "),
        (Sample::wheel_torque, Axis::Torque, TORQUE, false, "wheels "),
        (|s: &Sample| s.crank, Axis::Torque, TORQUE, true, "crank "),
    ] {
        lines.push(Line {
            points: run.line(value),
            axis,
            color,
            dashed,
            name: Some(name),
        });
    }
    // The shown run on top of the one compared with.
    let compared = lines.len() - 4;
    lines.rotate_left(compared);
    kit::section(
        ui,
        &format!("#{} {}, pulled in gear {}", run.number, run.car, run.gear),
    );
    chart(ui, &lines, 250.0);
    kit::note(
        ui,
        "Solid: at the wheels. Dashed: at the crank. Orange: power (hp, right). Blue: \
         torque (N·m, left); at the wheels, it's their power over the engine's speed.",
    );
    let (wheel_rpm, wheel_hp) = run.peak(Sample::wheel_hp);
    let (crank_rpm, crank_hp) = run.peak(Sample::crank_hp);
    let (wheel_torque_rpm, wheel_torque) = run.peak(Sample::wheel_torque);
    let (crank_torque_rpm, crank_torque) = run.peak(|s| s.crank);
    kit::info(
        ui,
        &format!("Wheels: {wheel_hp:.0} hp at {wheel_rpm:.0} rpm"),
        &format!("{wheel_torque:.0} N·m at {wheel_torque_rpm:.0} rpm"),
    );
    kit::info(
        ui,
        &format!("Crank: {crank_hp:.0} hp at {crank_rpm:.0} rpm"),
        &format!("{crank_torque:.0} N·m at {crank_torque_rpm:.0} rpm"),
    );
    if crank_hp > 0.0 {
        let lost = (1.0 - wheel_hp / crank_hp) * 100.0;
        kit::note(
            ui,
            &format!(
                "{lost:.0} % of the peak power doesn't reach the rollers: the gearbox and \
                 differentials, the tires, and spinning up the engine and wheels."
            ),
        );
    }
    if !run.broke.is_empty() {
        kit::warning(
            ui,
            &format!("Broke during the run: {}", run.broke.join(", ")),
        );
    }
    let run = &mut view.runs[i];
    ui.add(
        egui::TextEdit::singleline(&mut run.name)
            .hint_text("Name")
            .desired_width(f32::INFINITY)
            .font(FontId::proportional(16.0))
            .margin(egui::vec2(12.0, 8.0)),
    );
    ui.add(
        egui::TextEdit::multiline(&mut run.notes)
            .hint_text("Notes: what was changed, the weather, …")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
}

/// Every run, newest first: pick one to show; compare the shown one with another, or
/// delete it.
fn run_list(ui: &mut egui::Ui, view: &mut DynoView) {
    let mut delete = None;
    if let Some(shown) = view.shown.filter(|&i| i < view.runs.len()) {
        let mut options: Vec<Option<usize>> = vec![None];
        options.extend((0..view.runs.len()).rev().filter(|&k| k != shown).map(Some));
        if options.len() > 1 {
            let runs = &view.runs;
            kit::pick(
                ui,
                "Compare with",
                &mut view.compared,
                &options,
                |k| match k {
                    Some(k) => format!("#{} {}", runs[k].number, runs[k].name),
                    None => "nothing".to_string(),
                },
            );
        }
        if kit::button(ui, "Delete this run").clicked() {
            delete = Some(shown);
        }
    }
    kit::section(ui, &format!("Runs ({})", view.runs.len()));
    for (i, run) in view.runs.iter().enumerate().rev() {
        let (_, wheel) = run.peak(Sample::wheel_hp);
        let (_, crank) = run.peak(Sample::crank_hp);
        let mark = if view.shown == Some(i) { "● " } else { "" };
        let label = format!("{mark}#{} {}", run.number, run.name);
        let detail = format!("{}: {wheel:.0} whp / {crank:.0} hp", run.car);
        if kit::button_with(ui, &label, &detail).clicked() {
            view.shown = Some(i);
            if view.compared == Some(i) {
                view.compared = None;
            }
        }
    }
    let Some(i) = delete else {
        return;
    };
    view.runs.remove(i);
    let shift = |index: Option<usize>| match index {
        Some(k) if k == i => None,
        Some(k) if k > i => Some(k - 1),
        other => other,
    };
    view.shown = shift(view.shown).or(view.runs.len().checked_sub(1));
    view.compared = shift(view.compared).filter(|&k| Some(k) != view.shown);
}

/// The cam gears' timing and the wastegate's spring of the car on the rollers, applied
/// when asked: right away if the engine's maps are at hand, else once they're baked (a
/// cam's timing changes how the cylinders breathe), with the old engine running meanwhile.
/// The tune stays (it's what's to be retuned); the engine keeps its wear.
fn cams(ui: &mut egui::Ui, view: &mut DynoView) {
    let running = view.running();
    let Some(car) = &view.car else {
        return;
    };
    let fitted = &car.setup.engine.parts;
    let draft = view.draft.get_or_insert_with(|| fitted.clone());
    kit::slider(
        ui,
        "Intake cam advance",
        &mut draft.intake_timing,
        -10.0..=10.0,
        1.0,
        |v| format!("{v:.0}°"),
    );
    kit::slider(
        ui,
        "Exhaust cam retard",
        &mut draft.exhaust_timing,
        -10.0..=10.0,
        1.0,
        |v| format!("{v:.0}°"),
    );
    kit::note(
        ui,
        "Adjustable cam gears: advancing the intake cam fills better low down, retarding it \
         breathes better high up.",
    );
    ui.add_enabled_ui(draft.has_turbo(), |ui| {
        kit::slider(
            ui,
            "Wastegate opens at",
            &mut draft.boost,
            0.3..=1.5,
            0.05,
            |v| format!("{v:.2} bar"),
        );
    });
    let changed = draft != fitted;
    let busy = view.work.is_some() || running;
    let apply = ui
        .add_enabled_ui(changed && !busy, |ui| {
            kit::button(ui, "Apply (the tune stays)")
        })
        .inner
        .clicked();
    let revert = ui
        .add_enabled_ui(changed && !busy, |ui| kit::button(ui, "Back to how it is"))
        .inner
        .clicked();
    if revert {
        *draft = fitted.clone();
    }
    if let Some((.., work)) = &view.work {
        kit::progress(ui, "Baking the engine's maps…", work.progress());
    } else if running {
        kit::note(ui, "Not while the dyno runs.");
    } else if !view.status.is_empty() {
        kit::note(ui, &view.status);
    }
    if !apply {
        return;
    }
    let parts = draft.clone();
    let hardware = parts.hardware();
    let current = &car.setup.engine.build;
    let at_hand = if maps::fingerprint(&hardware) == maps::fingerprint(&current.hardware) {
        Some(current.maps.clone())
    } else {
        workshop::at_hand(&hardware)
    };
    match at_hand {
        Some(maps) => fit(view, parts, hardware, maps),
        None => {
            view.status.clear();
            view.work = Some((parts, hardware.clone(), Work::start(&hardware)));
        }
    }
}

/// Moves the baking on, and fits the engine once its maps are done.
fn bake(mut view: ResMut<DynoView>) {
    let Some((.., work)) = &mut view.work else {
        return;
    };
    let Some(baked) = work.poll() else {
        return;
    };
    let (parts, hardware, _) = view.work.take().expect("a job");
    match baked {
        Some(maps) => {
            workshop::keep(&hardware, &maps);
            fit(&mut view, parts, hardware, maps);
        }
        None => view.status = "Baking the maps failed.".to_string(),
    }
}

/// The car on the rollers with its engine built from `parts`, keeping its tune and wear.
fn fit(view: &mut DynoView, parts: Parts, hardware: EngineHardware, maps: EngineMaps) {
    let Some(car) = &mut view.car else {
        return;
    };
    let old = &car.setup.engine.build;
    let engine = EngineBuild {
        hardware,
        maps,
        ecu: old.ecu.clone(),
        octane: old.octane,
    };
    let damage = car.drivetrain.engine.damage;
    let setup = CarSetup::new(car.setup.build.clone(), parts, engine);
    *car = car.rebuilt(setup);
    car.drivetrain.engine.damage = damage;
    view.status = "Fitted.".to_string();
}
