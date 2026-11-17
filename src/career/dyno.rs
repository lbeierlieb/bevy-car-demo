//! The dyno shop, a scene of its own: the player's car strapped onto an all-wheel chassis
//! dyno (see `rollers`) in a room deep under the shop. A paid run warms up through the
//! gears and pulls in fourth while a screen shows the power at the wheels; then a window
//! shows the power and torque at the wheels and at the crank, and every run on file, each
//! with a name and notes to give it.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, RichText, Stroke};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hud::News;
use super::places::PlaceKind;
use super::rollers::{Rollers, STEP, Sample, Stage};
use super::{Career, PLAYER, Places, Ride, Scene, euros, spawn_ride};
use crate::audio::{EngineAudio, EngineSound};
use crate::camera::{ChaseCamera, OrbitInput};
use crate::car::{ActiveCar, Car};
use crate::dyno::{Axis, Line, chart};
use crate::terrain::Terrain;
use crate::visuals::spawn_model;

/// What a run costs (€).
pub const RUN_PRICE: f64 = 120.0;
/// A run needs this much fuel in the tank (L).
pub const RUN_FUEL: f32 = 1.0;
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
const RESULTS_ASIDE: f32 = 2.0;
/// The screen: how far in front of the camera, left of the middle, above it, and its size
/// (m).
const SCREEN_AHEAD: f32 = 3.4;
const SCREEN_LEFT: f32 = 1.55;
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
            .init_resource::<DynoBooked>()
            .add_systems(OnEnter(Scene::Dyno), (enter, build_room).chain())
            .add_systems(OnExit(Scene::Dyno), leave)
            .add_systems(
                Update,
                (run, (animate, look)).chain().run_if(in_state(Scene::Dyno)),
            )
            .add_systems(
                Update,
                hear.after(EngineSound).run_if(in_state(Scene::Dyno)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (screen, skip, results).run_if(in_state(Scene::Dyno)),
            );
    }
}

/// Whether the player paid for a run on the way in (or only came to look at the runs).
#[derive(Resource, Default)]
pub struct DynoBooked(pub bool);

/// A run on file: its number, the name and notes the player gave it, the car as it was
/// (name and engine), the gear it was pulled in, its samples, and what broke.
#[derive(Clone, Debug, PartialEq)]
pub struct DynoRun {
    pub number: u32,
    pub name: String,
    pub notes: String,
    pub car: String,
    pub gear: i8,
    pub samples: Vec<Sample>,
    pub broke: Vec<String>,
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

/// Everything in the room, which goes when the player leaves.
#[derive(Component)]
struct InDyno;

/// A roller, turning with the wheels on it; and the car's model on the rollers.
#[derive(Component)]
struct Roller;
#[derive(Component)]
struct CarModel;

/// The room and the car in it: where it is, the car on the rollers and its run, the
/// camera, the run shown in the results and the one compared with.
#[derive(Resource, Default)]
struct DynoView {
    origin: Vec3,
    car: Option<Car>,
    rollers: Option<Rollers>,
    /// Time not yet stepped (s), and whether the finished run is on file.
    owed: f32,
    filed: bool,
    swing: f32,
    eye: Option<Vec3>,
    /// How far the view has moved over for the results (0..1).
    aside: f32,
    screen: Transform,
    shown: Option<usize>,
    compared: Option<usize>,
}

impl DynoView {
    fn running(&self) -> bool {
        self.rollers
            .as_ref()
            .is_some_and(|rollers| rollers.stage != Stage::Done)
    }
}

/// Onto the rollers: the player's car comes off the road into the room.
#[allow(clippy::too_many_arguments)]
fn enter(
    mut commands: Commands,
    mut career: ResMut<Career>,
    places: Res<Places>,
    terrain: Res<Terrain>,
    booked: Res<DynoBooked>,
    mut view: ResMut<DynoView>,
    cars: Query<(Entity, &Car)>,
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
) {
    let shop = places.of(PlaceKind::Dyno).at;
    let ground = terrain.sample(shop.x, shop.y).0;
    view.origin = Vec3::new(shop.x, ground - DEPTH, shop.y);
    for (entity, car) in &cars {
        if car.index == PLAYER {
            career.bring_home(car);
        }
        commands.entity(entity).despawn();
    }
    let (setup, wear) = career.ride_car();
    let mut car = Car::spawn_at(PLAYER, setup, &terrain, shop.x, shop.y, 0.0);
    wear.put_on(&mut car);
    view.rollers = booked.0.then(|| Rollers::new(&mut car));
    view.car = Some(car);
    view.owed = 0.0;
    view.filed = false;
    view.swing = 0.0;
    view.eye = None;
    view.aside = 0.0;
    view.shown = career.runs.len().checked_sub(1);
    view.compared = None;
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
}

/// Off the rollers and back on the road in front of the shop.
#[allow(clippy::too_many_arguments)]
fn leave(
    mut commands: Commands,
    mut career: ResMut<Career>,
    places: Res<Places>,
    terrain: Res<Terrain>,
    mut view: ResMut<DynoView>,
    mut active: ResMut<ActiveCar>,
    things: Query<Entity, With<InDyno>>,
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
    view.rollers = None;
    if let Some(car) = view.car.take() {
        career.bring_home(&car);
    }
    let shop = places.of(PlaceKind::Dyno);
    spawn_ride(&mut commands, &career, &terrain, shop.at, shop.yaw);
    active.0 = PLAYER;
    career.save();
}

/// The room: a concrete cell with the rollers in the floor, the car on them strapped down,
/// a fan in front of it, lights, and the screen on a stand.
fn build_room(
    mut commands: Commands,
    view: Res<DynoView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
    let chassis = car.setup.build.chassis();
    let axles = [-0.5, 0.5].map(|side| side * chassis.wheelbase);
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
        block(
            &mut commands,
            at,
            Vec3::new(2.0 * w, 0.2, to - from),
            &concrete,
        );
    }
    let pit_half = ROLLER_LENGTH / 2.0 + 0.2;
    for z in axles {
        // The floor beside the slot, the pit's floor and walls.
        for side in [-1.0, 1.0] {
            let at = Vec3::new(side * (pit_half + w) / 2.0, -0.1, z);
            let size = Vec3::new(w - pit_half, 0.2, 2.0 * slot);
            block(&mut commands, at, size, &concrete);
            let at = Vec3::new(side * pit_half, -0.6, z);
            block(&mut commands, at, Vec3::new(0.05, 1.2, 2.0 * slot), &pit);
            let at = Vec3::new(0.0, -0.6, z + side * slot);
            block(
                &mut commands,
                at,
                Vec3::new(2.0 * pit_half, 1.2, 0.05),
                &pit,
            );
        }
        let at = Vec3::new(0.0, -1.2, z);
        block(
            &mut commands,
            at,
            Vec3::new(2.0 * pit_half, 0.1, 2.0 * slot),
            &pit,
        );
        // Yellow edges round the slot.
        for side in [-1.0, 1.0] {
            let at = Vec3::new(0.0, 0.004, z + side * (slot + 0.05));
            block(
                &mut commands,
                at,
                Vec3::new(2.0 * pit_half, 0.01, 0.1),
                &yellow,
            );
        }
    }
    // Walls and ceiling.
    block(
        &mut commands,
        Vec3::new(0.0, h + 0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * l),
        &wall,
    );
    for side in [-1.0, 1.0] {
        let at = Vec3::new(side * (w + 0.1), h / 2.0, 0.0);
        block(&mut commands, at, Vec3::new(0.2, h, 2.0 * l), &wall);
        let at = Vec3::new(0.0, h / 2.0, side * (l + 0.1));
        block(&mut commands, at, Vec3::new(2.0 * w, h, 0.2), &wall);
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
    // The car on them, facing the fan.
    let root = commands
        .spawn((
            InDyno,
            CarModel,
            Transform::from_translation(o + Vec3::Y * ROLLER_TOP),
            Visibility::default(),
        ))
        .id();
    spawn_model(
        &mut commands,
        root,
        &car.setup.build,
        &mut meshes,
        &mut materials,
    );
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
            block(&mut commands, foot - o, Vec3::new(0.16, 0.05, 0.16), &dark);
        }
    }
    // The fan in front, blowing at the radiator.
    let fan_z = axles[0] - 2.9;
    block(
        &mut commands,
        Vec3::new(0.0, 0.7, fan_z),
        Vec3::new(1.5, 1.4, 0.5),
        &dark,
    );
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
    let mut lights = Vec::new();
    for x in [-2.5, 2.5] {
        for z in [-3.5, 0.0, 3.5] {
            block(
                &mut commands,
                Vec3::new(x, h - 0.03, z),
                Vec3::new(0.25, 0.05, 2.2),
                &lamp,
            );
            lights.push(o + Vec3::new(x, h - 0.5, z));
        }
    }
    for at in lights {
        commands.spawn((
            InDyno,
            PointLight {
                intensity: 450_000.0,
                range: 20.0,
                ..default()
            },
            Transform::from_translation(at),
        ));
    }
}

/// Runs the dyno in real time (Space skips to the end), and files the run when it's done.
fn run(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<DynoView>,
    mut career: ResMut<Career>,
    mut news: ResMut<News>,
) {
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
    view.filed = true;
    career.bring_home(car);
    if !rollers.broke.is_empty() {
        news.say(format!("Bang! {}", rollers.broke.join(", ")));
    }
    if rollers.samples.len() < 2 {
        news.say("The run didn't get far enough to measure anything.");
        career.save();
        return;
    }
    let number = career.runs.iter().map(|run| run.number).max().unwrap_or(0) + 1;
    let setup = &car.setup;
    let owner = match career.ride {
        Ride::Mom => "Mom's ",
        Ride::Own(_) => "",
    };
    career.runs.push(DynoRun {
        number,
        name: format!("Run {number}"),
        notes: String::new(),
        car: format!(
            "{owner}{} · {}",
            setup.name, setup.engine.build.hardware.name
        ),
        gear: rollers.gear,
        samples: rollers.samples.clone(),
        broke: rollers.broke.clone(),
    });
    view.shown = Some(career.runs.len() - 1);
    career.save();
}

/// The rollers turn with the wheels; the car squats and shakes under power.
fn animate(
    time: Res<Time>,
    view: Res<DynoView>,
    mut rollers: Query<&mut Transform, (With<Roller>, Without<CarModel>)>,
    mut models: Query<&mut Transform, (With<CarModel>, Without<Roller>)>,
) {
    let (Some(car), Some(run)) = (&view.car, &view.rollers) else {
        return;
    };
    let tire = car.setup.axle(false).tire.radius;
    let angle = run.turned * tire / ROLLER_RADIUS;
    for mut transform in &mut rollers {
        transform.rotation = Quat::from_rotation_x(angle);
    }
    let load = car.drivetrain.load;
    let t = time.elapsed_secs();
    let shake = 0.0015 * load * ((t * 61.0).sin() + (t * 97.0).sin());
    for mut transform in &mut models {
        transform.translation = view.origin + Vec3::Y * (ROLLER_TOP + shake);
        transform.rotation = Quat::from_rotation_x(0.006 * load);
    }
}

/// The car's engine is what's heard, not the (missing) active car's.
fn hear(view: Res<DynoView>, audio: Res<EngineAudio>) {
    if let Some(car) = &view.car {
        audio.hear(&car.setup, &car.drivetrain);
    }
}

/// The camera on the car from in front and to its left, beside the screen; dragging
/// swings it round.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    orbit: Res<OrbitInput>,
    mut view: ResMut<DynoView>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if orbit.0 && buttons.pressed(MouseButton::Left) {
        view.swing = (view.swing - motion.delta.x * 0.005).clamp(-VIEW_SWING, VIEW_SWING);
    }
    let eye = view.origin + Quat::from_rotation_y(view.swing) * EYE;
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = view.eye.map_or(eye, |was| was.lerp(eye, ease));
    view.eye = Some(now);
    // With the results up on the right, the car moves over to the left.
    let aside = if view.running() { 0.0 } else { 1.0 };
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
    let mut corners = Vec::new();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let at = s.translation + s.right() * x * half.x + s.up() * y * half.y;
        let Ok(p) = camera.world_to_viewport(eye, at) else {
            return Ok(());
        };
        corners.push(egui::pos2(p.x, p.y));
    }
    let rect = egui::Rect::from_points(&corners);
    let ctx = contexts.ctx_mut()?;
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
        "ROLLWERK  AWD DYNO",
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
    let power = match run {
        Some(run) if run.stage == Stage::Done || run.stage == Stage::Coast => peak,
        Some(run) => run.power.max(0.0) / crate::dyno::HORSEPOWER,
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
        if matches!(run.map(|run| run.stage), Some(Stage::Done | Stage::Coast)) {
            "PEAK WHP"
        } else {
            "WHP"
        },
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
    painter.text(
        egui::pos2(left, rect.top() + 0.5 * h),
        Align2::LEFT_TOP,
        format!(
            "GEAR {gear}  {:>5.0} RPM  {speed:>3.0} KM/H",
            if drivetrain.running {
                drivetrain.rpm
            } else {
                0.0
            }
        ),
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
    if !view.running() {
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

/// After the run: the shown run's chart and peaks, its name and notes, every run on file,
/// and running again or driving out.
fn results(
    mut contexts: EguiContexts,
    mut view: ResMut<DynoView>,
    mut career: ResMut<Career>,
    mut news: ResMut<News>,
    mut scene: ResMut<NextState<Scene>>,
) -> Result {
    if view.running() {
        return Ok(());
    }
    let mut again = false;
    let mut out = false;
    egui::Window::new("Dyno runs")
        .anchor(Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .default_width(620.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.set_width(620.0);
            shown_run(ui, &mut view, &mut career);
            ui.separator();
            run_list(ui, &mut view, &mut career);
            ui.separator();
            let car = view.car.as_ref();
            let why_not = car.and_then(|car| cannot_run(car, career.money));
            ui.horizontal(|ui| {
                let label = if view.rollers.is_some() {
                    "Run it again"
                } else {
                    "Run it"
                };
                let text = format!("{label}: {}", euros(RUN_PRICE));
                again = ui
                    .add_enabled(why_not.is_none(), egui::Button::new(text))
                    .clicked();
                out = ui.button(RichText::new("Drive out").strong()).clicked();
                if let Some(why) = why_not {
                    ui.label(RichText::new(why).weak());
                }
            });
        });
    if again && career.pay(RUN_PRICE) {
        let view = &mut *view;
        if let Some(car) = &mut view.car {
            view.rollers = Some(Rollers::new(car));
            view.owed = 0.0;
            view.filed = false;
        }
        career.save();
        news.say(format!("Another run: {}.", euros(RUN_PRICE)));
    }
    if out {
        scene.set(Scene::Driving);
    }
    Ok(())
}

/// Why `car` can't have a run with `money`, if it can't.
pub fn cannot_run(car: &Car, money: f64) -> Option<&'static str> {
    if car
        .drivetrain
        .engine
        .blown(&car.setup.engine.build.hardware)
    {
        Some("The engine won't run: it needs a repair at home.")
    } else if car.fuel.litres() < RUN_FUEL {
        Some("There isn't enough fuel in the tank for a run.")
    } else if money < RUN_PRICE {
        Some("Not enough money for a run.")
    } else {
        None
    }
}

/// The shown run: power and torque at the wheels (solid) and at the crank (dashed), the
/// peaks, and its name and notes to edit.
fn shown_run(ui: &mut egui::Ui, view: &mut DynoView, career: &mut Career) {
    let Some(i) = view.shown.filter(|&i| i < career.runs.len()) else {
        ui.label("No runs on file yet.");
        return;
    };
    let mut lines = Vec::new();
    if let Some(other) = view.compared.and_then(|k| career.runs.get(k)) {
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
    let run = &career.runs[i];
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
    ui.horizontal(|ui| {
        ui.strong(format!("#{}", run.number));
        ui.label(&run.car);
        ui.label(RichText::new(format!("pulled in gear {}", run.gear)).weak());
    });
    chart(ui, &lines, 250.0);
    ui.label(
        RichText::new(
            "Solid: at the wheels. Dashed: at the crank. Orange: power (hp, right). Blue: \
             torque (N·m, left); at the wheels, it's their power over the engine's speed.",
        )
        .small()
        .weak(),
    );
    let (wheel_rpm, wheel_hp) = run.peak(Sample::wheel_hp);
    let (crank_rpm, crank_hp) = run.peak(Sample::crank_hp);
    let (wheel_torque_rpm, wheel_torque) = run.peak(Sample::wheel_torque);
    let (crank_torque_rpm, crank_torque) = run.peak(|s| s.crank);
    egui::Grid::new("dyno peaks").show(ui, |ui| {
        ui.label("");
        ui.strong("Power");
        ui.strong("Torque");
        ui.end_row();
        ui.colored_label(POWER, "Wheels");
        ui.label(format!("{wheel_hp:.0} hp at {wheel_rpm:.0} rpm"));
        ui.label(format!(
            "{wheel_torque:.0} N·m at {wheel_torque_rpm:.0} rpm"
        ));
        ui.end_row();
        ui.colored_label(POWER, "Crank");
        ui.label(format!("{crank_hp:.0} hp at {crank_rpm:.0} rpm"));
        ui.label(format!(
            "{crank_torque:.0} N·m at {crank_torque_rpm:.0} rpm"
        ));
        ui.end_row();
    });
    if crank_hp > 0.0 {
        let lost = (1.0 - wheel_hp / crank_hp) * 100.0;
        ui.label(
            RichText::new(format!(
                "{lost:.0} % of the peak power doesn't reach the rollers: the gearbox and \
                 differentials, the tires, and spinning up the engine and wheels."
            ))
            .weak(),
        );
    }
    if !run.broke.is_empty() {
        ui.colored_label(
            Color32::from_rgb(255, 90, 70),
            format!("Broke during the run: {}", run.broke.join(", ")),
        );
    }
    let run = &mut career.runs[i];
    let mut edited = false;
    ui.horizontal(|ui| {
        ui.label("Name");
        let name = ui.add(egui::TextEdit::singleline(&mut run.name).desired_width(400.0));
        edited |= name.lost_focus();
    });
    let notes = ui.add(
        egui::TextEdit::multiline(&mut run.notes)
            .hint_text("Notes: what was changed, the weather, …")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    edited |= notes.lost_focus();
    if edited {
        career.save();
    }
}

/// Every run on file, newest first: pick one to show, another to compare with, or delete.
fn run_list(ui: &mut egui::Ui, view: &mut DynoView, career: &mut Career) {
    ui.strong(format!("Runs on file ({})", career.runs.len()));
    let mut delete = None;
    egui::ScrollArea::vertical()
        .max_height(150.0)
        .show(ui, |ui| {
            egui::Grid::new("dyno runs").striped(true).show(ui, |ui| {
                for (i, run) in career.runs.iter().enumerate().rev() {
                    let shown = view.shown == Some(i);
                    let text = format!("#{} {}", run.number, run.name);
                    if ui.selectable_label(shown, text).clicked() {
                        view.shown = Some(i);
                        if view.compared == Some(i) {
                            view.compared = None;
                        }
                    }
                    ui.label(RichText::new(&run.car).weak());
                    let (_, wheel) = run.peak(Sample::wheel_hp);
                    let (_, crank) = run.peak(Sample::crank_hp);
                    ui.label(format!("{wheel:.0} whp / {crank:.0} hp"));
                    let mut compared = view.compared == Some(i);
                    if ui
                        .add_enabled(!shown, egui::Checkbox::new(&mut compared, "compare"))
                        .changed()
                    {
                        view.compared = compared.then_some(i);
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
    let Some(i) = delete else {
        return;
    };
    career.runs.remove(i);
    let shift = |index: Option<usize>| match index {
        Some(k) if k == i => None,
        Some(k) if k > i => Some(k - 1),
        other => other,
    };
    view.shown = shift(view.shown).or(career.runs.len().checked_sub(1));
    view.compared = shift(view.compared).filter(|&k| Some(k) != view.shown);
    career.save();
}
