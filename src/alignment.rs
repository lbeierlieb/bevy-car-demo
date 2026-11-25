//! The alignment shops: the car on a four-post alignment lift in a bay deep under the shop,
//! a measuring head clamped on each wheel, red laser beams between them (along each side
//! and across in front of and behind the car), and the aligner's console, whose screen
//! shows each wheel's camber and toe and its tire pressure against the spec. The window on
//! the right sets them (`garage::alignment`), on the driven car as they're moved; the car
//! on the lift shows its wheels lean and turn with them, and the heads tilt along.
//!
//! The driven car stays parked on the shop's marker meanwhile (a ghost, its input held
//! like in the dyno's room); the car on the lift is drawn from its looks.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Stroke};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::audio::EngineAudio;
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car};
use crate::chassis::FACTORY_TOE;
use crate::garage;
use crate::looks::{Looks, Motion};
use crate::menu::kit::{self, Panel, Place};
use crate::shops::{self, AtShop, SceneBack, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;

/// How far under the ground at the shop the bay is (m).
const DEPTH: f32 = 220.0;
/// The bay's half width (across the car) and half length, and its height (m).
const HALF_WIDTH: f32 = 7.5;
const HALF_LENGTH: f32 = 9.0;
const HEIGHT: f32 = 4.8;
/// The lift: how high the runways' tops are, their width and thickness (m).
const RACK: f32 = 0.95;
const RUNWAY_WIDTH: f32 = 0.62;
const RUNWAY_DEPTH: f32 = 0.2;
/// A measuring head: its size (across, up, along) and how far out from the clamp's middle
/// its shaft holds it (m).
const HEAD: Vec3 = Vec3::new(0.1, 0.17, 0.32);
const SHAFT: f32 = 0.14;
/// The cross beams run this far over the runways (m), under the bumpers.
const BEAM_HEIGHT: f32 = 0.06;
const BEAM_THICKNESS: f32 = 0.012;
/// The camera's place relative to the car's middle (in front and to its left), what it
/// looks at, how far it may swing round (rad), and how near and far it may zoom (a share of
/// its distance).
const EYE: Vec3 = Vec3::new(-4.4, 2.1, -3.4);
const TARGET: Vec3 = Vec3::new(0.0, RACK + 0.25, -0.2);
const VIEW_SWING: f32 = 1.4;
const ZOOM: std::ops::RangeInclusive<f32> = 0.45..=1.3;
/// How far right of the car the camera looks, with the window up on the right (m).
const ASIDE: f32 = 0.5;
/// The console's screen: how far in front of the camera, left of the middle, above it,
/// and its size (m).
const SCREEN_AHEAD: f32 = 2.6;
const SCREEN_LEFT: f32 = 0.92;
const SCREEN_UP: f32 = 0.15;
const SCREEN_SIZE: Vec2 = Vec2::new(0.88, 0.55);
/// The spec's tolerances either side: camber, toe per wheel (degrees) and pressure (bar).
const CAMBER_TOLERANCE: f32 = 0.5;
const TOE_TOLERANCE: f32 = 0.1;
const PRESSURE_TOLERANCE: f32 = 0.15;
const IN_SPEC: Color32 = Color32::from_rgb(90, 230, 110);
const OFF_SPEC: Color32 = Color32::from_rgb(255, 80, 60);
const SCREEN_INK: Color32 = Color32::from_rgb(220, 235, 245);

pub struct AlignmentPlugin;

impl Plugin for AlignmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bay>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (show, rig, look.after(crate::camera::follow)).run_if(in_bay),
                )
                    .chain(),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (shop, (screen, window).run_if(in_bay)),
            );
    }
}

/// The bay, while the car is in it: where it is, the camera's swing and zoom, and where
/// the console's screen is.
#[derive(Resource, Default)]
struct Bay {
    inside: bool,
    origin: Vec3,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
    aside: f32,
    screen: Transform,
}

fn in_bay(bay: Res<Bay>) -> bool {
    bay.inside
}

/// Everything in the bay, which goes when the car leaves it.
#[derive(Component)]
struct InBay;

/// The car on the lift, drawn from the driven car's `Looks` and `Motion`.
#[derive(Component)]
struct CarModel;

/// What moves with a wheel's alignment: its clamp (tilting with the wheel), its head
/// (hanging level from the clamp's shaft), or a laser beam between two heads' ports.
#[derive(Component, Clone, Copy)]
enum Rig {
    Clamp(usize),
    Head(usize),
    Beam(Port, Port),
}

/// Where a beam leaves a head (of a corner: front left, front right, rear left, rear
/// right): the end facing the other head on its side, or the tip of its arm, down by the
/// runway, facing across the car.
#[derive(Clone, Copy)]
enum Port {
    Side(usize),
    Tip(usize),
}

/// A corner's side (+1 right, -1 left), and which way its head's arm reaches (-1 forward
/// for the front heads, +1 back for the rear ones).
fn side(corner: usize) -> f32 {
    if corner.is_multiple_of(2) { -1.0 } else { 1.0 }
}
fn reach(corner: usize) -> f32 {
    if corner < 2 { -1.0 } else { 1.0 }
}

/// The shop's window (outside the bay): onto the lift.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut bay: ResMut<Bay>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if bay.inside {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Alignment, |ui, _| {
        enter = kit::button(ui, "Onto the alignment lift").clicked();
        kit::note(
            ui,
            "A four-post alignment lift with laser heads on the wheels: camber, toe, tire \
             pressures and the brakes.",
        );
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    *bay = Bay {
        inside: true,
        origin: Vec3::new(marker.x, ground - DEPTH, marker.y),
        zoom: 1.0,
        ..default()
    };
    let eye = bay.origin + EYE;
    let ahead = (bay.origin + TARGET - eye).normalize();
    let right = ahead.cross(Vec3::Y).normalize();
    let at = eye + ahead * SCREEN_AHEAD - right * SCREEN_LEFT + Vec3::Y * SCREEN_UP;
    bay.screen = Transform::from_translation(at).looking_at(eye.with_y(at.y), Vec3::Y);
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.9, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    build_bay(&mut commands, &bay, car, &mut meshes, &mut materials);
    Ok(())
}

/// Off the lift and back to the driven car, parked at the shop (the alignment is on it
/// already).
fn leave(
    commands: &mut Commands,
    bay: &mut Bay,
    things: &Query<Entity, With<InBay>>,
    (away, audio): (&mut Away, &mut EngineAudio),
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
    bay.inside = false;
    away.0 = false;
    audio.hushed = false;
}

/// The car left the shop (or the server, and with it the shop) while in the bay: out.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut bay: ResMut<Bay>,
    things: Query<Entity, With<InBay>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if bay.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut bay, &things, parts, &mut cameras);
    }
}

/// The car on the lift, standing straight ahead: its looks and its suspension as the
/// driven car's, the front wheels pointing ahead and nothing turning.
fn looks_on_lift(car: &Car) -> (Looks, Motion) {
    let mut motion = Motion::of(car);
    for wheel in &mut motion.wheels {
        (wheel.steer, wheel.spin) = (0.0, 0.0);
    }
    (Looks::of(&car.setup), motion)
}

/// Where the car on the lift is (its centre of mass, as `Looks` are drawn from): on the
/// runways, midway between its axles over the bay's middle, facing -z.
fn model_at(bay: &Bay, looks: &Looks) -> Transform {
    Transform::from_translation(bay.origin + Vec3::Y * RACK - looks.body_offset)
}

/// A wheel's clamp (in the wheel's plane on its outside, tilting with it) and its head
/// (hanging level from the clamp's shaft), in the bay.
fn wheel_rig(looks: &Looks, motion: &Motion, model: &Transform, corner: usize) -> [Transform; 2] {
    let pose = looks.corner_pose(corner, &motion.wheels[corner]);
    let s = side(corner);
    let width = looks.build.wheels[corner / 2].tire.width_m();
    let clamp = pose.wheel_center + pose.upright * Vec3::new(s * (width / 2.0 + 0.03), 0.0, 0.0);
    let shaft_end = clamp + pose.upright * Vec3::new(s * SHAFT, 0.0, 0.0);
    let head = shaft_end + Vec3::new(s * HEAD.x / 2.0, 0.0, 0.0);
    [
        Transform::from_translation(model.transform_point(clamp))
            .with_rotation(model.rotation * pose.upright),
        Transform::from_translation(model.transform_point(head)).with_rotation(model.rotation),
    ]
}

/// How far each head's arm reaches along the car from the head's middle, to clear the
/// tire (m).
fn arm_length(looks: &Looks, corner: usize) -> f32 {
    (looks.axles[corner / 2].radius + 0.28).max(0.55)
}

/// Where a beam leaves its head, from the head's transform.
fn port_at(port: Port, heads: &[Transform; 4], arms: &[f32; 4], origin: Vec3) -> Vec3 {
    match port {
        Port::Side(corner) => {
            let along = -reach(corner) * (HEAD.z / 2.0 + 0.01);
            heads[corner].transform_point(Vec3::new(0.0, 0.02, along))
        }
        Port::Tip(corner) => {
            let head = heads[corner];
            let tip = Vec3::new(-side(corner) * 0.03, 0.0, reach(corner) * arms[corner]);
            head.transform_point(tip)
                .with_y(origin.y + RACK + BEAM_HEIGHT)
        }
    }
}

/// The bay: a bright workshop with the lift (four posts, two runways, turn plates under
/// the front wheels and slip plates under the rear, ramps at the back), the car on it with
/// a head on each wheel and the beams between them, the aligner's console with its screen,
/// and lights.
fn build_bay(
    commands: &mut Commands,
    bay: &Bay,
    car: &Car,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let o = bay.origin;
    let (looks, motion) = looks_on_lift(car);
    let model = model_at(bay, &looks);
    let mut material = |color: Color, roughness: f32, metallic: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic,
            ..default()
        })
    };
    let floor = material(Color::srgb(0.42, 0.45, 0.46), 0.35, 0.0);
    let wall = material(Color::srgb(0.78, 0.79, 0.80), 0.9, 0.0);
    let skirting = material(Color::srgb(0.22, 0.30, 0.42), 0.8, 0.0);
    let lift = material(Color::srgb(0.70, 0.08, 0.06), 0.5, 0.2);
    let runway = material(Color::srgb(0.30, 0.31, 0.33), 0.6, 0.6);
    let steel = material(Color::srgb(0.72, 0.73, 0.75), 0.3, 0.9);
    let yellow = material(Color::srgb(0.95, 0.75, 0.10), 0.6, 0.0);
    let dark = material(Color::srgb(0.07, 0.07, 0.08), 0.5, 0.3);
    let head_body = material(Color::srgb(0.16, 0.17, 0.19), 0.4, 0.4);
    let console = material(Color::srgb(0.12, 0.24, 0.42), 0.5, 0.2);
    let chest = material(Color::srgb(0.65, 0.07, 0.07), 0.4, 0.3);
    let glow = |materials: &mut Assets<StandardMaterial>, color: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: Color::from(color),
            emissive: color,
            unlit: true,
            ..default()
        })
    };
    let laser = glow(materials, LinearRgba::rgb(9.0, 0.15, 0.1));
    let display = glow(materials, LinearRgba::rgb(0.2, 2.0, 0.6));
    let lamp = glow(materials, LinearRgba::rgb(8.0, 8.0, 7.5));
    let glass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.02, 0.03, 0.05),
        emissive: LinearRgba::rgb(0.02, 0.04, 0.06),
        perceptual_roughness: 0.2,
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let block = |commands: &mut Commands, at: Vec3, size: Vec3, material: &Handle<_>| {
        commands.spawn((
            InBay,
            Mesh3d(cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(o + at).with_scale(size),
        ));
    };
    let (w, l, h) = (HALF_WIDTH, HALF_LENGTH, HEIGHT);
    // The floor, walls (a dark band along their foot) and ceiling.
    block(
        commands,
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * l),
        &floor,
    );
    block(
        commands,
        Vec3::new(0.0, h + 0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * l),
        &wall,
    );
    for s in [-1.0, 1.0] {
        let at = Vec3::new(s * (w + 0.1), h / 2.0, 0.0);
        block(commands, at, Vec3::new(0.2, h, 2.0 * l), &wall);
        let at = Vec3::new(0.0, h / 2.0, s * (l + 0.1));
        block(commands, at, Vec3::new(2.0 * w, h, 0.2), &wall);
        let at = Vec3::new(s * (w - 0.01), 0.6, 0.0);
        block(commands, at, Vec3::new(0.04, 1.2, 2.0 * l), &skirting);
        let at = Vec3::new(0.0, 0.6, s * (l - 0.01));
        block(commands, at, Vec3::new(2.0 * w, 1.2, 0.04), &skirting);
    }
    // The axles and the runways' middles, in the bay's frame (the car's middle at the
    // origin, facing -z).
    let setup = &car.setup;
    let middle = (setup.front_z + setup.rear_z) / 2.0;
    let axles = [setup.front_z - middle, setup.rear_z - middle];
    let track = setup.build.chassis().track;
    let lane = (track[0] + track[1]) / 4.0;
    let (front_end, rear_end) = (axles[0] - 1.3, axles[1] + 1.1);
    let length = rear_end - front_end;
    let along = (front_end + rear_end) / 2.0;
    // Yellow lines round the lift on the floor.
    let (line_x, line_z) = (
        lane + RUNWAY_WIDTH / 2.0 + 0.9,
        (front_end - 0.6, rear_end + 2.4),
    );
    for s in [-1.0, 1.0] {
        let at = Vec3::new(s * line_x, 0.003, (line_z.0 + line_z.1) / 2.0);
        block(
            commands,
            at,
            Vec3::new(0.1, 0.006, line_z.1 - line_z.0),
            &yellow,
        );
    }
    for z in [line_z.0, line_z.1] {
        let at = Vec3::new(0.0, 0.003, z);
        block(
            commands,
            at,
            Vec3::new(2.0 * line_x + 0.1, 0.006, 0.1),
            &yellow,
        );
    }
    // The runways, with yellow edges, a turn plate under each front wheel and a slip plate
    // under each rear one; ramps up at the back.
    let turn_plate = meshes.add(Cylinder::new(0.24, 0.02));
    let turn_ring = meshes.add(Torus::new(0.17, 0.185));
    for s in [-1.0, 1.0] {
        let x = s * lane;
        let at = Vec3::new(x, RACK - RUNWAY_DEPTH / 2.0, along);
        block(
            commands,
            at,
            Vec3::new(RUNWAY_WIDTH, RUNWAY_DEPTH, length),
            &runway,
        );
        for edge in [-1.0, 1.0] {
            let at = Vec3::new(x + edge * (RUNWAY_WIDTH / 2.0 - 0.02), RACK + 0.01, along);
            block(commands, at, Vec3::new(0.04, 0.02, length), &yellow);
        }
        let plate = o + Vec3::new(x, RACK + 0.006, axles[0]);
        commands.spawn((
            InBay,
            Mesh3d(turn_plate.clone()),
            MeshMaterial3d(steel.clone()),
            Transform::from_translation(plate),
        ));
        commands.spawn((
            InBay,
            Mesh3d(turn_ring.clone()),
            MeshMaterial3d(dark.clone()),
            Transform::from_translation(plate + Vec3::Y * 0.011),
        ));
        let at = Vec3::new(x, RACK + 0.006, axles[1]);
        block(commands, at, Vec3::new(0.5, 0.012, 0.55), &steel);
        // The ramp from the floor up to the runway's back end.
        let (foot, top) = (
            Vec3::new(x, 0.0, rear_end + 1.9),
            Vec3::new(x, RACK - 0.04, rear_end),
        );
        let ramp_length = foot.distance(top);
        commands.spawn((
            InBay,
            Mesh3d(cube.clone()),
            MeshMaterial3d(runway.clone()),
            Transform::from_translation(o + (foot + top) / 2.0)
                .looking_at(o + top, Vec3::Y)
                .with_scale(Vec3::new(RUNWAY_WIDTH, 0.08, ramp_length)),
        ));
    }
    // The posts and the cross beams carrying the runways.
    let post_x = lane + RUNWAY_WIDTH / 2.0 + 0.35;
    for z in [front_end - 0.15, rear_end + 0.15] {
        for s in [-1.0, 1.0] {
            let at = Vec3::new(s * post_x, 1.2, z);
            block(commands, at, Vec3::new(0.26, 2.4, 0.26), &lift);
            let at = Vec3::new(s * post_x, 0.01, z);
            block(commands, at, Vec3::new(0.5, 0.02, 0.5), &dark);
        }
        let at = Vec3::new(0.0, RACK - RUNWAY_DEPTH - 0.1, z);
        block(
            commands,
            at,
            Vec3::new(2.0 * post_x + 0.2, 0.22, 0.3),
            &lift,
        );
    }
    // The car on it.
    commands.spawn((InBay, CarModel, looks.clone(), motion, model));
    // A head on each wheel: the clamp's cross on the rim and its shaft; the head with its
    // display, and its arm reaching along the car and down to the runway, with the emitter
    // for the beam across.
    let bar = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let shaft = meshes.add(Cylinder::new(0.018, SHAFT));
    let mut arms = [0.0; 4];
    let mut heads = [Transform::IDENTITY; 4];
    for corner in 0..4 {
        let [clamp, head] = wheel_rig(&looks, &motion, &model, corner);
        heads[corner] = head;
        arms[corner] = arm_length(&looks, corner);
        let (s, r) = (side(corner), reach(corner));
        let radius = looks.axles[corner / 2].radius;
        let span = 2.0 * radius * 0.62;
        commands
            .spawn((InBay, Rig::Clamp(corner), clamp, Visibility::default()))
            .with_children(|parent| {
                for size in [Vec3::new(0.03, span, 0.035), Vec3::new(0.03, 0.035, span)] {
                    parent.spawn((
                        Mesh3d(bar.clone()),
                        MeshMaterial3d(steel.clone()),
                        Transform::from_scale(size),
                    ));
                }
                // Claws at the rim's edge, top, bottom and either side.
                for angle in [0.0, 1.0, 2.0, 3.0] {
                    let at = Quat::from_rotation_x(angle * std::f32::consts::FRAC_PI_2)
                        * Vec3::new(0.0, span / 2.0, 0.0);
                    parent.spawn((
                        Mesh3d(bar.clone()),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_translation(at - Vec3::X * s * 0.02)
                            .with_scale(Vec3::splat(0.05)),
                    ));
                }
                parent.spawn((
                    Mesh3d(shaft.clone()),
                    MeshMaterial3d(steel.clone()),
                    Transform::from_translation(Vec3::X * s * SHAFT / 2.0)
                        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ));
            });
        let drop = head.translation.y - (o.y + RACK + BEAM_HEIGHT);
        let arm = arms[corner];
        commands
            .spawn((InBay, Rig::Head(corner), head, Visibility::default()))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(head_body.clone()),
                    Transform::from_scale(HEAD),
                ));
                // The display on its outer side, and a handle on top.
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(display.clone()),
                    Transform::from_xyz(s * (HEAD.x / 2.0 + 0.002), 0.03, 0.0)
                        .with_scale(Vec3::new(0.004, 0.05, 0.11)),
                ));
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(dark.clone()),
                    Transform::from_xyz(0.0, HEAD.y / 2.0 + 0.03, 0.0)
                        .with_scale(Vec3::new(0.025, 0.025, 0.16)),
                ));
                // The arm: along the car, then down.
                let from = HEAD.z / 2.0;
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(steel.clone()),
                    Transform::from_xyz(0.0, -0.04, r * (from + arm) / 2.0).with_scale(Vec3::new(
                        0.035,
                        0.035,
                        arm - from,
                    )),
                ));
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(steel.clone()),
                    Transform::from_xyz(0.0, -0.04 - drop / 2.0, r * arm)
                        .with_scale(Vec3::new(0.03, drop, 0.03)),
                ));
                parent.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(head_body.clone()),
                    Transform::from_xyz(-s * 0.01, -drop, r * arm)
                        .with_scale(Vec3::new(0.07, 0.06, 0.07)),
                ));
            });
    }
    // The beams: along each side, and across in front and behind.
    let beams = [
        (Port::Side(0), Port::Side(2)),
        (Port::Side(1), Port::Side(3)),
        (Port::Tip(0), Port::Tip(1)),
        (Port::Tip(2), Port::Tip(3)),
    ];
    for (a, b) in beams {
        let transform = beam(port_at(a, &heads, &arms, o), port_at(b, &heads, &arms, o));
        commands.spawn((
            InBay,
            Rig::Beam(a, b),
            Mesh3d(bar.clone()),
            MeshMaterial3d(laser.clone()),
            transform,
            NotShadowCaster,
        ));
    }
    // The aligner's console under its screen: a cabinet with a keyboard shelf and a pole.
    let screen = bay.screen;
    let foot = screen.translation.with_y(o.y) - screen.forward() * 0.25;
    let turn = Transform::from_translation(foot).with_rotation(screen.rotation);
    let piece = |commands: &mut Commands, at: Vec3, size: Vec3, material: &Handle<_>| {
        commands.spawn((
            InBay,
            Mesh3d(cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(turn.transform_point(at))
                .with_rotation(screen.rotation)
                .with_scale(size),
        ));
    };
    piece(
        commands,
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::new(0.9, 1.0, 0.6),
        &console,
    );
    piece(
        commands,
        Vec3::new(0.0, 0.92, 0.38),
        Vec3::new(0.8, 0.04, 0.3),
        &dark,
    );
    let pole = screen.translation.y - o.y - 1.0;
    piece(
        commands,
        Vec3::new(0.0, 1.0 + pole / 2.0, -0.1),
        Vec3::new(0.08, pole, 0.08),
        &dark,
    );
    commands.spawn((
        InBay,
        Mesh3d(cube.clone()),
        MeshMaterial3d(glass),
        screen.with_scale(Vec3::new(SCREEN_SIZE.x + 0.08, SCREEN_SIZE.y + 0.08, 0.05)),
    ));
    // A tool chest by the right wall, and a stand of spare heads' chargers on the left.
    block(
        commands,
        Vec3::new(w - 0.45, 0.55, 2.5),
        Vec3::new(0.6, 1.1, 1.4),
        &chest,
    );
    block(
        commands,
        Vec3::new(w - 0.45, 1.12, 2.5),
        Vec3::new(0.62, 0.04, 1.42),
        &dark,
    );
    for k in 0..4 {
        let at = Vec3::new(w - 0.4, 0.4 + 0.25 * k as f32, 2.5);
        block(commands, at, Vec3::new(0.62, 0.01, 1.3), &dark);
    }
    // Lights in the ceiling.
    for x in [-2.6, 2.6] {
        for z in [-4.5, -1.5, 1.5, 4.5] {
            let at = Vec3::new(x, h - 0.03, z);
            block(commands, at, Vec3::new(0.25, 0.05, 2.2), &lamp);
            commands.spawn((
                InBay,
                PointLight {
                    intensity: 420_000.0,
                    range: 22.0,
                    ..default()
                },
                Transform::from_translation(o + Vec3::new(x, h - 0.5, z)),
            ));
        }
    }
}

/// A beam from `a` to `b`.
fn beam(a: Vec3, b: Vec3) -> Transform {
    let length = a.distance(b).max(1e-3);
    Transform::from_translation((a + b) / 2.0)
        .looking_at(b, Vec3::Y)
        .with_scale(Vec3::new(BEAM_THICKNESS, BEAM_THICKNESS, length))
}

/// The car on the lift as the driven car is now (its alignment set in the window).
fn show(
    bay: Res<Bay>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut models: Query<(&mut Looks, &mut Motion, &mut Transform), With<CarModel>>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let (looks, motion) = looks_on_lift(car);
    for (mut shown, mut moved, mut transform) in &mut models {
        *transform = model_at(&bay, &looks);
        shown.set_if_neq(looks.clone());
        moved.set_if_neq(motion);
    }
}

/// The clamps tilt and turn with their wheels, the heads hang from them, and the beams
/// follow the heads.
fn rig(
    bay: Res<Bay>,
    models: Query<(&Looks, &Motion, &Transform), With<CarModel>>,
    mut parts: Query<(&Rig, &mut Transform), Without<CarModel>>,
) {
    let Ok((looks, motion, model)) = models.single() else {
        return;
    };
    let rigs: [[Transform; 2]; 4] =
        std::array::from_fn(|corner| wheel_rig(looks, motion, model, corner));
    let heads = rigs.map(|[_, head]| head);
    let arms = std::array::from_fn(|corner| arm_length(looks, corner));
    for (rig, mut transform) in &mut parts {
        *transform = match *rig {
            Rig::Clamp(corner) => rigs[corner][0],
            Rig::Head(corner) => rigs[corner][1],
            Rig::Beam(a, b) => beam(
                port_at(a, &heads, &arms, bay.origin),
                port_at(b, &heads, &arms, bay.origin),
            ),
        };
    }
}

/// The camera on the car from in front and to its left, beside the console; dragging
/// swings it round, the mouse wheel takes it nearer or farther.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut bay: ResMut<Bay>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        bay.swing = (bay.swing - motion.delta.x * 0.005).clamp(-VIEW_SWING, VIEW_SWING);
    }
    let zoom = bay.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    bay.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = bay.origin + TARGET;
    let offset = Quat::from_rotation_y(bay.swing) * (EYE - TARGET) * bay.zoom;
    let eye = target + offset;
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = bay.eye.map_or(eye, |was| was.lerp(eye, ease));
    bay.eye = Some(now);
    bay.aside += (1.0 - bay.aside) * ease * 0.5;
    let right = (target - now).cross(Vec3::Y).normalize();
    let looked_at = target + right * ASIDE * bay.aside * bay.zoom;
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(looked_at, Vec3::Y);
    }
}

/// A reading on the aligner's screen: its value, the spec's middle and tolerance, and
/// how it's written.
struct Reading {
    name: &'static str,
    value: f32,
    spec: f32,
    tolerance: f32,
    unit: &'static str,
    decimals: usize,
}

impl Reading {
    fn in_spec(&self) -> bool {
        (self.value - self.spec).abs() <= self.tolerance + 1e-4
    }
}

/// Each wheel's readings: camber and toe (degrees, toe-in positive) as the heads measure
/// them, and the tire's cold pressure, each against the spec (the camber the suspension kit
/// is aligned to, the factory's toe, and the pressure the tires suit standing).
fn readings(car: &Car, looks: &Looks, motion: &Motion) -> [[Reading; 3]; 4] {
    let build = &car.setup.build;
    let loads = garage::static_loads(&car.setup);
    std::array::from_fn(|corner| {
        let axle = corner / 2;
        let pose = looks.corner_pose(corner, &motion.wheels[corner]);
        let wheel = &build.wheels[axle];
        let suited = wheel
            .tire
            .params(build.rubber(axle))
            .suited_pressure(loads[axle]);
        [
            Reading {
                name: "CAMBER",
                value: pose.camber.to_degrees(),
                spec: build.kit_camber(axle),
                tolerance: CAMBER_TOLERANCE,
                unit: "°",
                decimals: 2,
            },
            Reading {
                name: "TOE",
                value: build.toe[axle],
                spec: FACTORY_TOE[axle],
                tolerance: TOE_TOLERANCE,
                unit: "°",
                decimals: 2,
            },
            Reading {
                name: "PRESSURE",
                value: build.pressure[axle],
                spec: suited,
                tolerance: PRESSURE_TOLERANCE,
                unit: " bar",
                decimals: 2,
            },
        ]
    })
}

/// The aligner's screen: the car from above with each wheel's camber, toe and pressure
/// beside it, a gauge each with the spec's band in green, and the total toe per axle.
fn screen(
    mut contexts: EguiContexts,
    bay: Res<Bay>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    models: Query<(&Looks, &Motion), With<CarModel>>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let (Ok((camera, eye)), Ok((looks, motion))) = (cameras.single(), models.single()) else {
        return Ok(());
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    // Where the screen's corners are in the window.
    let s = bay.screen;
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
        Color32::from_rgb(10, 18, 32),
        Stroke::new(1.0, Color32::from_rgb(50, 70, 90)),
    ));
    let (w, h) = (rect.width(), rect.height());
    let at = |x: f32, y: f32| rect.min + egui::vec2(x * w, y * h);
    let font = |size: f32| FontId::monospace(crate::menu::scene_text(size * h));
    let dim = SCREEN_INK.gamma_multiply(0.55);
    painter.text(
        at(0.03, 0.03),
        Align2::LEFT_TOP,
        "WHEEL ALIGNMENT",
        font(0.06),
        dim,
    );
    painter.text(
        at(0.97, 0.03),
        Align2::RIGHT_TOP,
        car.setup.name.to_uppercase(),
        font(0.06),
        SCREEN_INK,
    );
    // The car from above, in the middle.
    let body = egui::Rect::from_min_max(at(0.43, 0.15), at(0.57, 0.95));
    painter.rect_stroke(
        body,
        0.08 * w,
        Stroke::new(1.5, dim),
        egui::StrokeKind::Inside,
    );
    let readings = readings(car, looks, motion);
    for (corner, reading) in readings.iter().enumerate() {
        let (right, rear) = (corner % 2 == 1, corner >= 2);
        // The wheel, tilted by its toe (exaggerated tenfold, as aligners draw it).
        let centre = at(
            if right { 0.585 } else { 0.415 },
            if rear { 0.80 } else { 0.30 },
        );
        let toe = reading[1].value.to_radians() * 10.0 * if right { -1.0 } else { 1.0 };
        let (sin, cos) = toe.sin_cos();
        let (hw, hh) = (0.022 * w, 0.075 * h);
        let tire: Vec<egui::Pos2> = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .into_iter()
            .map(|(x, y)| centre + egui::vec2(x * cos - y * sin, x * sin + y * cos))
            .collect();
        let all_in = reading.iter().all(Reading::in_spec);
        painter.add(egui::Shape::convex_polygon(
            tire,
            if all_in { IN_SPEC } else { OFF_SPEC }.gamma_multiply(0.7),
            Stroke::NONE,
        ));
        // Its readings beside it, outside the car.
        let x0 = if right { 0.63 } else { 0.03 };
        let top = if rear { 0.56 } else { 0.13 };
        let label = ["FRONT LEFT", "FRONT RIGHT", "REAR LEFT", "REAR RIGHT"][corner];
        painter.text(at(x0, top), Align2::LEFT_TOP, label, font(0.045), dim);
        for (k, reading) in reading.iter().enumerate() {
            let y = top + 0.07 + 0.11 * k as f32;
            gauge(
                &painter,
                egui::Rect::from_min_max(at(x0, y), at(x0 + 0.34, y + 0.1)),
                reading,
                h,
            );
        }
    }
    // The total toe per axle, between the wheels.
    for (axle, y) in [(0, 0.47), (1, 0.97)] {
        let total = 2.0 * car.setup.build.toe[axle];
        let spec = 2.0 * FACTORY_TOE[axle];
        let color = if (total - spec).abs() <= 2.0 * TOE_TOLERANCE + 1e-4 {
            IN_SPEC
        } else {
            OFF_SPEC
        };
        painter.text(
            at(0.5, y),
            Align2::CENTER_BOTTOM,
            format!("TOTAL {total:+.2}°"),
            font(0.042),
            color,
        );
    }
    Ok(())
}

/// A reading: its name and value, and a bar with the spec's band in the middle and a
/// needle at the value.
fn gauge(painter: &egui::Painter, rect: egui::Rect, reading: &Reading, h: f32) {
    let font = |size: f32| FontId::monospace(crate::menu::scene_text(size * h));
    let color = if reading.in_spec() { IN_SPEC } else { OFF_SPEC };
    let d = reading.decimals;
    painter.text(
        rect.left_top(),
        Align2::LEFT_TOP,
        reading.name,
        font(0.04),
        SCREEN_INK.gamma_multiply(0.7),
    );
    painter.text(
        rect.right_top(),
        Align2::RIGHT_TOP,
        format!("{:+.d$}{}", reading.value, reading.unit),
        font(0.05),
        color,
    );
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 0.6 * rect.height()),
        rect.right_bottom(),
    );
    painter.rect_filled(bar, 1.0, Color32::from_rgb(40, 20, 24));
    // The bar spans the spec ± three tolerances; the band is the spec ± one.
    let span = 3.0 * reading.tolerance;
    let x = |value: f32| {
        let share = ((value - reading.spec) / span * 0.5 + 0.5).clamp(0.0, 1.0);
        egui::lerp(bar.left()..=bar.right(), share)
    };
    let band = egui::Rect::from_x_y_ranges(
        x(reading.spec - reading.tolerance)..=x(reading.spec + reading.tolerance),
        bar.y_range(),
    );
    painter.rect_filled(band, 0.0, Color32::from_rgb(30, 110, 50));
    let needle = x(reading.value);
    painter.line_segment(
        [
            egui::pos2(needle, bar.top() - 0.1 * bar.height()),
            egui::pos2(needle, bar.bottom() + 0.1 * bar.height()),
        ],
        Stroke::new(2.0, SCREEN_INK),
    );
}

/// The aligner's window, on the right: camber, toe, pressures and the brakes, set on the
/// driven car as they're moved; and back to the car.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut bay: ResMut<Bay>,
    things: Query<Entity, With<InBay>>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    (mut cameras, mut back): (Query<(&ChaseCamera, &mut Projection)>, ResMut<SceneBack>),
) -> Result {
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let mut out = back.take();
    let ctx = contexts.ctx_mut()?;
    Panel::new("alignment", "Alignment")
        .place(Place::Right)
        .width(440.0)
        .back(Some("Back to the car"))
        .show(ctx, |ui| {
            if kit::button(ui, "Back to the car").clicked() {
                out = true;
            }
            let mut build = car.setup.build.clone();
            garage::alignment(ui, &mut build, &car.setup);
            shops::rebuild(&mut car, build);
            kit::note(
                ui,
                "The screen shows each wheel against the spec: green within it. Drag to look \
                 round, the mouse wheel to go nearer.",
            );
        });
    if out {
        leave(
            &mut commands,
            &mut bay,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}
