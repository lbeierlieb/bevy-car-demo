//! Helicopters: the rescue helicopter that carries a car back to the road when the driver
//! asks for it (`rescue`), and the one that follows the hunted in a car hunt, 50 m above
//! them (`Chase`, flown by the server and sent to everyone in the round).
//!
//! A helicopter on screen is an entity with a `Heli` (its cable and what hangs on it) and a
//! `Transform` (its centre, forward along −z); `HeliPlugin` gives it its model, turns the
//! rotors, flashes the beacons and draws the cable. The ones flown somewhere else (another
//! player's rescue, the hunt's) carry a `Flown` with their latest pose, and are drawn where
//! that pose says they are by now. A helicopter is never fogged, and one beyond the far
//! plane is drawn at its edge, shrunk to the size it would look (with a beacon that stays a
//! few pixels big), so a hunter sees the hunted's from anywhere.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

use crate::looks::Looks;
use crate::net::Connection;
use crate::net::client::RemoteCar;

pub mod rescue;

/// The hook under the belly, in the helicopter's frame.
pub const HOOK: Vec3 = Vec3::new(0.0, -1.3, 0.0);
/// The main rotor's radius and how fast it turns (rad/s), and the tail rotor's.
const ROTOR_RADIUS: f32 = 5.1;
const ROTOR_RATE: f32 = 41.0;
const TAIL_ROTOR_RADIUS: f32 = 0.85;
const TAIL_ROTOR_RATE: f32 = 190.0;
/// Where the main rotor's hub and the tail rotor's are.
const HUB: Vec3 = Vec3::new(0.0, 1.55, -0.1);
const TAIL_HUB: Vec3 = Vec3::new(0.22, 0.75, 6.55);
/// Something farther than this is drawn at this distance, shrunk to look as big (m; the
/// camera's far plane is at 1000 m).
const DRAWN_WITHIN: f32 = 900.0;
/// The beacons are at least this big on screen (radians across).
const BEACON_ANGLE: f32 = 0.004;
/// How long a flown helicopter's pose is carried on at its velocity without news (s), and
/// the time constant it's drawn closing on where that is (s).
const PREDICT: f32 = 0.6;
const SMOOTHING: f32 = 0.15;
/// A flown helicopter not heard of for this long is gone (s).
const GONE: f64 = 2.0;

/// The car hunt's helicopter: how high above the hunted it holds (m), the least it stays
/// above the ground under it (m), and how it chases: the speed it adds per metre it's off
/// (1/s), how quickly it takes up the velocity it wants (s), the most it accelerates
/// sideways and up or down (m/s²), how fast it turns (rad/s), and how far it wanders
/// about its spot when the hunted is slow (m). No top speed: it can't be outrun.
pub const CHASE_HEIGHT: f32 = 50.0;
const CHASE_CLEARANCE: f32 = 30.0;
const CHASE_GAIN: f32 = 0.35;
const CHASE_RESPONSE: f32 = 1.2;
const CHASE_ACCELERATION: f32 = 7.0;
const CHASE_CLIMB: f32 = 4.0;
const CHASE_TURN: f32 = 1.0;
const CHASE_WANDER: f32 = 6.0;
/// How far the rotor tilts per m/s² it pushes with, and to hold against the air per m/s
/// (rad, rad·s/m), and at most (rad).
const DRAG_TILT: f32 = 0.025;
const MOST_TILT: f32 = 0.45;

pub struct HeliPlugin;

impl Plugin for HeliPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Parts>().add_systems(
            PostUpdate,
            (
                (follow_remote_rescues, follow_the_hunt).run_if(resource_exists::<Connection>),
                fly_flown,
                build_models,
                (spin, flash, hang_cables, keep_in_view).chain(),
            )
                .chain()
                .before(TransformSystems::Propagate),
        );
    }
}

/// Where a helicopter is and how it moves: as sent, at `time` on the sender's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeliPose {
    pub time: f64,
    pub position: Vec3,
    pub velocity: Vec3,
    pub rotation: Quat,
    /// How far the cable is let down from the hook (m).
    pub cable: f32,
    /// Whether a car hangs from it (the cable then goes down to that car's roof).
    pub hooked: bool,
}

/// A helicopter on screen.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Heli {
    /// How far the cable is let down from the hook (m).
    pub cable: f32,
    /// Where it's hooked on, if anywhere (a car's roof).
    pub load: Option<Vec3>,
}

/// A helicopter flown elsewhere: its latest pose, the clock it's on against ours (theirs
/// less ours, s), and the car it's hooked to (an entity whose roof the cable goes to).
#[derive(Component, Clone, Copy, Debug)]
pub struct Flown {
    pub pose: HeliPose,
    pub heard: f64,
    pub load: Option<Entity>,
    drawn: Option<Vec3>,
}

impl Flown {
    fn new(pose: HeliPose, heard: f64) -> Self {
        Self {
            pose,
            heard,
            load: None,
            drawn: None,
        }
    }
}

/// The rescue helicopter of a remote car (another player's).
#[derive(Component)]
struct RescueOf(Entity);

/// The car hunt's helicopter.
#[derive(Component)]
struct HuntHeli;

/// A spinning part: about its own y axis, this fast (rad/s).
#[derive(Component)]
struct Spin(f32);

/// A beacon (scaled up with distance so it stays visible), and its rest size (m).
#[derive(Component)]
struct Beacon(f32);

/// The cable and the hook block at its end.
#[derive(Component)]
struct Cable;
#[derive(Component)]
struct HookBlock;

/// The model's meshes and materials, made once.
#[derive(Resource, Default)]
struct Parts {
    made: Option<Made>,
}

struct Made {
    ball: Handle<Mesh>,
    cube: Handle<Mesh>,
    rod: Handle<Mesh>,
    disc: Handle<Mesh>,
    boom: Handle<Mesh>,
    paint: Handle<StandardMaterial>,
    stripe: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    blur: Handle<StandardMaterial>,
    beacon: Handle<StandardMaterial>,
    beacon_off: Handle<StandardMaterial>,
}

impl Made {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut material = |color: Color, roughness: f32, metallic: f32| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: roughness,
                metallic,
                fog_enabled: false,
                ..default()
            })
        };
        let paint = material(Color::srgb(0.78, 0.08, 0.06), 0.35, 0.1);
        let stripe = material(Color::srgb(0.93, 0.93, 0.9), 0.4, 0.0);
        let glass = material(Color::srgb(0.05, 0.08, 0.12), 0.08, 0.6);
        let dark = material(Color::srgb(0.08, 0.08, 0.09), 0.6, 0.2);
        let metal = material(Color::srgb(0.55, 0.57, 0.6), 0.45, 0.7);
        let glow = |color: Color| StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 40.0,
            unlit: true,
            fog_enabled: false,
            ..default()
        };
        Self {
            ball: meshes.add(Sphere::new(1.0).mesh().uv(24, 14)),
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            rod: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(8)),
            disc: meshes.add(Cylinder::new(1.0, 0.01).mesh().resolution(32)),
            boom: meshes.add(ConicalFrustum {
                radius_top: 0.16,
                radius_bottom: 0.42,
                height: 1.0,
            }),
            paint,
            stripe,
            glass,
            dark,
            metal,
            blur: materials.add(StandardMaterial {
                base_color: Color::srgba(0.1, 0.1, 0.1, 0.13),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                fog_enabled: false,
                ..default()
            }),
            beacon: materials.add(glow(Color::srgb(1.0, 0.1, 0.05))),
            beacon_off: materials.add(StandardMaterial {
                base_color: Color::srgb(0.3, 0.02, 0.02),
                unlit: true,
                fog_enabled: false,
                ..default()
            }),
        }
    }
}

/// A unit cylinder (radius 1, height 1, along y) stretched into a rod from `a` to `b`.
fn rod(a: Vec3, b: Vec3, radius: f32) -> Transform {
    let d = b - a;
    Transform {
        translation: (a + b) / 2.0,
        rotation: Quat::from_rotation_arc(Vec3::Y, d.normalize_or(Vec3::Y)),
        scale: Vec3::new(radius, d.length().max(1e-3), radius),
    }
}

/// Every new helicopter gets its model.
fn build_models(
    mut commands: Commands,
    helis: Query<Entity, Added<Heli>>,
    mut parts: ResMut<Parts>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if helis.is_empty() {
        return;
    }
    let m = parts
        .made
        .get_or_insert_with(|| Made::new(&mut meshes, &mut materials));
    for entity in &helis {
        commands
            .entity(entity)
            .insert(Visibility::default())
            .with_children(|heli| {
                let mut part = |mesh: &Handle<Mesh>,
                                material: &Handle<StandardMaterial>,
                                transform: Transform| {
                    heli.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        transform,
                    ));
                };
                let ellipsoid =
                    |at: Vec3, half: Vec3| Transform::from_translation(at).with_scale(half);
                // The cabin, its glazed nose and the engine fairing on top.
                part(
                    &m.ball,
                    &m.paint,
                    ellipsoid(Vec3::new(0.0, 0.0, -0.2), Vec3::new(0.9, 0.95, 2.1)),
                );
                part(
                    &m.ball,
                    &m.stripe,
                    ellipsoid(Vec3::new(0.0, -0.25, -0.15), Vec3::new(0.92, 0.35, 1.95)),
                );
                part(
                    &m.ball,
                    &m.glass,
                    ellipsoid(Vec3::new(0.0, 0.2, -1.35), Vec3::new(0.78, 0.68, 1.05)),
                );
                part(
                    &m.ball,
                    &m.glass,
                    ellipsoid(Vec3::new(0.0, 0.25, -0.2), Vec3::new(0.93, 0.45, 0.6)),
                );
                part(
                    &m.ball,
                    &m.paint,
                    ellipsoid(Vec3::new(0.0, 0.85, 0.35), Vec3::new(0.6, 0.45, 1.5)),
                );
                // The tail boom, fin, stabiliser with its end plates.
                part(
                    &m.boom,
                    &m.paint,
                    Transform::from_translation(Vec3::new(0.0, 0.45, 3.9))
                        .with_rotation(Quat::from_rotation_x(-FRAC_PI_2 + 0.04))
                        .with_scale(Vec3::new(1.0, 5.4, 1.0)),
                );
                part(
                    &m.cube,
                    &m.paint,
                    Transform::from_translation(Vec3::new(0.0, 1.05, 6.55))
                        .with_rotation(Quat::from_rotation_x(-0.35))
                        .with_scale(Vec3::new(0.1, 1.5, 0.75)),
                );
                part(
                    &m.cube,
                    &m.stripe,
                    Transform::from_translation(Vec3::new(0.0, 0.5, 5.3))
                        .with_scale(Vec3::new(2.1, 0.06, 0.5)),
                );
                for side in [-1.0, 1.0] {
                    part(
                        &m.cube,
                        &m.paint,
                        Transform::from_translation(Vec3::new(side * 1.05, 0.55, 5.3))
                            .with_scale(Vec3::new(0.05, 0.45, 0.45)),
                    );
                }
                // The skids, each on two struts.
                for side in [-1.0, 1.0] {
                    let x = side * 1.05;
                    let y = -1.45;
                    part(
                        &m.rod,
                        &m.metal,
                        rod(Vec3::new(x, y, -1.6), Vec3::new(x, y, 1.6), 0.05),
                    );
                    part(
                        &m.rod,
                        &m.metal,
                        rod(Vec3::new(x, y, -1.6), Vec3::new(x, y + 0.3, -2.05), 0.05),
                    );
                    for z in [-0.9, 0.75] {
                        part(
                            &m.rod,
                            &m.metal,
                            rod(Vec3::new(x, y, z), Vec3::new(side * 0.55, -0.7, z), 0.045),
                        );
                    }
                }
                // The mast, the hub, and the hook.
                part(&m.rod, &m.dark, rod(Vec3::new(0.0, 1.2, -0.1), HUB, 0.1));
                part(
                    &m.cube,
                    &m.dark,
                    Transform::from_translation(HOOK + 0.1 * Vec3::Y)
                        .with_scale(Vec3::new(0.25, 0.2, 0.25)),
                );
                // Beacons on the fairing and under the belly.
                for (at, size) in [
                    (Vec3::new(0.0, 1.3, 1.6), 0.12),
                    (Vec3::new(0.0, -0.98, 0.5), 0.1),
                ] {
                    heli.spawn((
                        Beacon(size),
                        Mesh3d(m.ball.clone()),
                        MeshMaterial3d(m.beacon.clone()),
                        Transform::from_translation(at).with_scale(Vec3::splat(size)),
                        NotShadowCaster,
                    ));
                }
                // The main rotor: four blades and the blur of them turning.
                heli.spawn((
                    Spin(ROTOR_RATE),
                    Transform::from_translation(HUB),
                    Visibility::default(),
                ))
                .with_children(|rotor| {
                    for k in 0..4 {
                        let angle = k as f32 * FRAC_PI_2;
                        let along = Quat::from_rotation_y(angle) * Vec3::X;
                        rotor.spawn((
                            Mesh3d(m.cube.clone()),
                            MeshMaterial3d(m.dark.clone()),
                            Transform::from_translation(along * ROTOR_RADIUS / 2.0)
                                .with_rotation(
                                    Quat::from_rotation_y(angle) * Quat::from_rotation_x(0.08),
                                )
                                .with_scale(Vec3::new(ROTOR_RADIUS, 0.04, 0.3)),
                        ));
                    }
                    rotor.spawn((
                        Mesh3d(m.ball.clone()),
                        MeshMaterial3d(m.dark.clone()),
                        Transform::from_scale(Vec3::new(0.3, 0.15, 0.3)),
                    ));
                });
                heli.spawn((
                    Mesh3d(m.disc.clone()),
                    MeshMaterial3d(m.blur.clone()),
                    Transform::from_translation(HUB).with_scale(Vec3::new(
                        ROTOR_RADIUS,
                        1.0,
                        ROTOR_RADIUS,
                    )),
                    NotShadowCaster,
                ));
                // The tail rotor, turning about the x axis.
                heli.spawn((
                    Transform::from_translation(TAIL_HUB)
                        .with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
                    Visibility::default(),
                ))
                .with_children(|tail| {
                    tail.spawn((
                        Spin(TAIL_ROTOR_RATE),
                        Transform::default(),
                        Visibility::default(),
                    ))
                    .with_children(|rotor| {
                        rotor.spawn((
                            Mesh3d(m.cube.clone()),
                            MeshMaterial3d(m.dark.clone()),
                            Transform::from_scale(Vec3::new(2.0 * TAIL_ROTOR_RADIUS, 0.03, 0.14)),
                        ));
                    });
                    tail.spawn((
                        Mesh3d(m.disc.clone()),
                        MeshMaterial3d(m.blur.clone()),
                        Transform::from_scale(Vec3::new(TAIL_ROTOR_RADIUS, 1.0, TAIL_ROTOR_RADIUS)),
                        NotShadowCaster,
                    ));
                });
                // The cable (stretched every frame) and the hook block at its end.
                heli.spawn((
                    Cable,
                    Mesh3d(m.rod.clone()),
                    MeshMaterial3d(m.dark.clone()),
                    Transform::from_translation(HOOK),
                    Visibility::Hidden,
                ));
                heli.spawn((
                    HookBlock,
                    Mesh3d(m.cube.clone()),
                    MeshMaterial3d(m.metal.clone()),
                    Transform::from_translation(HOOK).with_scale(Vec3::new(0.18, 0.3, 0.18)),
                ));
            });
    }
}

/// The rotors turn.
fn spin(time: Res<Time>, mut parts: Query<(&Spin, &mut Transform)>) {
    for (spin, mut transform) in &mut parts {
        transform.rotate_local_y(spin.0 * time.delta_secs());
    }
}

/// The beacons flash once a second, and are scaled up with distance so they stay a few
/// pixels across.
fn flash(
    time: Res<Time>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    parts: Res<Parts>,
    helis: Query<&GlobalTransform, With<Heli>>,
    mut beacons: Query<(
        &Beacon,
        &ChildOf,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let Some(m) = &parts.made else {
        return;
    };
    let eye = cameras.iter().next().map(|c| c.translation());
    let on = time.elapsed_secs().fract() < 0.18;
    for (beacon, child_of, mut transform, mut material) in &mut beacons {
        let wanted = if on { &m.beacon } else { &m.beacon_off };
        if material.0 != *wanted {
            material.0 = wanted.clone();
        }
        let Ok(heli) = helis.get(child_of.parent()) else {
            continue;
        };
        // The heli may be drawn shrunk (see `keep_in_view`): its scale is how much.
        let shrunk = heli.scale().x.max(1e-6);
        let distance = eye.map_or(0.0, |eye| eye.distance(heli.translation())) / shrunk;
        let size = beacon.0.max(distance * BEACON_ANGLE / 2.0);
        transform.scale = Vec3::splat(size);
    }
}

/// The cable goes down from the hook as far as it's let down, or to the car on it; the
/// hook block hangs at its end.
#[allow(clippy::type_complexity)]
fn hang_cables(
    helis: Query<(&Heli, &Transform, &Children)>,
    mut cables: Query<(&mut Transform, &mut Visibility), (With<Cable>, Without<Heli>)>,
    mut blocks: Query<&mut Transform, (With<HookBlock>, Without<Heli>, Without<Cable>)>,
) {
    for (heli, at, children) in &helis {
        // In the helicopter's frame (it's never scaled while a car hangs from it: that
        // only happens far away).
        let end = match heli.load {
            Some(load) => at.rotation.inverse() * (load - at.translation),
            None => HOOK + (at.rotation.inverse() * Vec3::NEG_Y) * heli.cable,
        };
        let length = end.distance(HOOK);
        for &child in children {
            if let Ok((mut transform, mut visibility)) = cables.get_mut(child) {
                *transform = rod(HOOK, end, 0.02);
                *visibility = if length > 0.05 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            if let Ok(mut transform) = blocks.get_mut(child) {
                transform.translation = end + (end - HOOK).normalize_or(Vec3::NEG_Y) * -0.15;
                transform.rotation =
                    Quat::from_rotation_arc(Vec3::NEG_Y, (end - HOOK).normalize_or(Vec3::NEG_Y));
            }
        }
    }
}

/// A helicopter beyond `DRAWN_WITHIN` is drawn there, along the same line of sight, shrunk
/// to look as big as it would. (Its `Transform` is put back where it is first thing next
/// frame by whoever flies it: `fly_flown` or the rescue.)
fn keep_in_view(
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut helis: Query<&mut Transform, With<Heli>>,
) {
    let Some(eye) = cameras.iter().next().map(|c| c.translation()) else {
        return;
    };
    for mut transform in &mut helis {
        let offset = transform.translation - eye;
        let distance = offset.length();
        let scale = if distance > DRAWN_WITHIN {
            DRAWN_WITHIN / distance
        } else {
            1.0
        };
        transform.translation = eye + offset * scale;
        transform.scale = Vec3::splat(scale);
    }
}

/// Helicopters flown elsewhere are drawn where their latest pose says they are by now,
/// closing on that smoothly; a scripted car's rescue (`CarState::scripted`) exactly as of
/// when the car is shown, between its poses, so the cable stays on the roof. Ones not
/// heard of for a while go.
fn fly_flown(
    mut commands: Commands,
    time: Res<Time<Real>>,
    connection: Option<Res<Connection>>,
    roofs: Query<(&Transform, &Looks, Option<&RemoteCar>), Without<Heli>>,
    mut helis: Query<(Entity, &mut Flown, &mut Heli, &mut Transform)>,
) {
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    let their = connection.and_then(|c| c.server_time(now));
    for (entity, mut flown, mut heli, mut transform) in &mut helis {
        if now - flown.heard > GONE {
            commands.entity(entity).despawn();
            continue;
        }
        let scripted = (flown.load)
            .and_then(|car| roofs.get(car).ok()?.2)
            .filter(|car| car.latest().is_some_and(|state| state.scripted))
            .and_then(|car| car.heli_at(car.shown_at()));
        let pose = scripted.unwrap_or(flown.pose);
        if scripted.is_some() {
            flown.drawn = Some(pose.position);
            transform.translation = pose.position;
            transform.rotation = pose.rotation;
        } else {
            let ahead = their.map_or(0.0, |t| {
                (t - pose.time).clamp(0.0, f64::from(PREDICT)) as f32
            });
            let wanted = pose.position + pose.velocity * ahead;
            let position = match flown.drawn {
                Some(drawn) if drawn.distance(wanted) < 30.0 => {
                    let moved = drawn + pose.velocity * dt;
                    moved.lerp(wanted, 1.0 - (-dt / SMOOTHING).exp())
                }
                _ => wanted,
            };
            flown.drawn = Some(position);
            transform.translation = position;
            transform.rotation = transform
                .rotation
                .slerp(pose.rotation, 1.0 - (-dt / SMOOTHING).exp());
        }
        transform.scale = Vec3::ONE;
        heli.cable = pose.cable;
        heli.load = (flown.load.filter(|_| pose.hooked))
            .and_then(|car| roofs.get(car).ok())
            .map(|(at, looks, _)| at.transform_point(roof(looks)));
    }
}

/// Where a car is hooked on: the middle of its roof, in its chassis frame.
pub fn roof(looks: &Looks) -> Vec3 {
    let hull = &looks.hull;
    hull.center + hull.axes[1] * hull.half.y
}

/// Every remote car that's being rescued has its helicopter drawn (the other player's
/// game flies it and sends its pose with the car's state).
fn follow_remote_rescues(
    mut commands: Commands,
    time: Res<Time<Real>>,
    cars: Query<(Entity, &RemoteCar)>,
    mut helis: Query<(Entity, &RescueOf, &mut Flown)>,
) {
    let now = time.elapsed_secs_f64();
    for (car, remote) in &cars {
        let Some(pose) = remote.latest().and_then(|state| state.heli) else {
            continue;
        };
        match helis.iter_mut().find(|(_, of, _)| of.0 == car) {
            Some((_, _, mut flown)) => {
                if pose.time > flown.pose.time {
                    flown.pose = pose;
                    flown.heard = now;
                }
            }
            None => {
                commands.spawn((
                    Heli::default(),
                    Flown {
                        load: Some(car),
                        ..Flown::new(pose, now)
                    },
                    RescueOf(car),
                    Transform::from_translation(pose.position).with_rotation(pose.rotation),
                ));
            }
        }
    }
    // A rescue that's over (the car's states have none any more) goes at once.
    for (entity, of, _) in &helis {
        let over = cars.get(of.0).map_or(true, |(_, remote)| {
            remote.latest().is_some_and(|state| state.heli.is_none())
        });
        if over {
            commands.entity(entity).despawn();
        }
    }
}

/// The car hunt's helicopter, while the server sends where it is (and only in a round
/// under way: it goes with the round).
fn follow_the_hunt(
    mut commands: Commands,
    time: Res<Time<Real>>,
    connection: Res<Connection>,
    mut helis: Query<(Entity, &mut Flown), With<HuntHeli>>,
) {
    let now = time.elapsed_secs_f64();
    let current = (connection.hunt_heli)
        .filter(|&(_, heard)| now - heard < GONE && connection.in_running_round());
    let Some((pose, heard)) = current else {
        for (entity, _) in &helis {
            commands.entity(entity).despawn();
        }
        return;
    };
    match helis.iter_mut().next() {
        Some((_, mut flown)) => {
            if pose.time > flown.pose.time {
                flown.pose = pose;
                flown.heard = heard;
            }
        }
        None => {
            commands.spawn((
                Heli::default(),
                Flown::new(pose, heard),
                HuntHeli,
                Transform::from_translation(pose.position).with_rotation(pose.rotation),
            ));
        }
    }
}

/// A helicopter's attitude: facing `yaw` (rad about y, 0 facing −z), its rotor tilted
/// towards where it pushes, `push` (m/s², without gravity), within `MOST_TILT`.
pub fn attitude(yaw: f32, push: Vec3) -> Quat {
    let heading = Quat::from_rotation_y(yaw);
    let lift = push + Vec3::Y * 9.81;
    let up = lift.normalize_or(Vec3::Y);
    let tilt = up.angle_between(Vec3::Y).min(MOST_TILT);
    let axis = Vec3::Y.cross(up);
    let tilted = if axis.length_squared() > 1e-8 {
        Quat::from_axis_angle(axis.normalize(), tilt)
    } else {
        Quat::IDENTITY
    };
    tilted * heading
}

/// What it pushes with to fly at `velocity` changing by `acceleration`: the change and
/// holding against the air.
pub fn push(velocity: Vec3, acceleration: Vec3) -> Vec3 {
    let drag = Vec3::new(velocity.x, 0.0, velocity.z) * (DRAG_TILT * 9.81);
    acceleration + drag
}

/// Turns `yaw` towards `wanted` by at most `step` (rad).
pub fn turn(yaw: f32, wanted: f32, step: f32) -> f32 {
    let difference = (wanted - yaw + PI).rem_euclid(TAU) - PI;
    yaw + difference.clamp(-step, step)
}

/// The yaw of a heading in the xz plane (0 facing −z).
pub fn yaw_of(direction: Vec2) -> f32 {
    (-direction.x).atan2(-direction.y)
}

/// The car hunt's helicopter, flown on the server: it holds `CHASE_HEIGHT` above the
/// hunted, chasing them with no top speed but a limit on how hard it accelerates, so it
/// lags when they're quick off the mark and overshoots when they brake hard, then turns
/// back. Where the hunted is slow it wanders about a little, as a pilot holding over a
/// spot does.
#[derive(Clone, Copy, Debug)]
pub struct Chase {
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    /// What it pushed with in the last step (m/s², for its tilt).
    push: Vec3,
    /// How long it has flown (s, for the wandering).
    time: f32,
}

impl Chase {
    /// Above `target`, facing along `yaw`.
    pub fn new(target: Vec3, yaw: f32) -> Self {
        Self {
            position: target + Vec3::Y * CHASE_HEIGHT,
            velocity: Vec3::ZERO,
            yaw,
            push: Vec3::ZERO,
            time: 0.0,
        }
    }

    /// Flies `dt` on after the hunted, at `target` moving at `target_velocity`, over land
    /// `ground` high under it.
    pub fn step(&mut self, target: Vec3, target_velocity: Vec3, ground: f32, dt: f32) {
        self.time += dt;
        let t = self.time;
        // A slow target: hold over it, drifting about the spot.
        let slow = (1.0 - target_velocity.xz().length() / 8.0).clamp(0.0, 1.0);
        let wander = CHASE_WANDER * slow * Vec3::new((0.21 * t).sin(), 0.0, (0.13 * t + 1.0).sin());
        let aim = target + wander;
        let off = Vec2::new(aim.x - self.position.x, aim.z - self.position.z);
        let wanted = target_velocity.xz() + off * CHASE_GAIN;
        let flat = self.velocity.xz();
        let change = ((wanted - flat) / CHASE_RESPONSE).clamp_length_max(CHASE_ACCELERATION);
        let height = (target.y + CHASE_HEIGHT).max(ground + CHASE_CLEARANCE);
        let climb_wanted = target_velocity.y.clamp(-10.0, 10.0) + (height - self.position.y) * 0.5;
        let climb =
            ((climb_wanted - self.velocity.y) / CHASE_RESPONSE).clamp(-CHASE_CLIMB, CHASE_CLIMB);
        let acceleration = Vec3::new(change.x, climb, change.y);
        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;
        // Never into the ground, whatever.
        self.position.y = self.position.y.max(ground + 10.0);
        let flat = self.velocity.xz();
        if flat.length() > 4.0 {
            self.yaw = turn(self.yaw, yaw_of(flat), CHASE_TURN * dt);
        }
        let smoothing = 1.0 - (-dt / 0.3).exp();
        self.push = self.push.lerp(push(self.velocity, acceleration), smoothing);
    }

    /// How it's tilted and turned.
    pub fn rotation(&self) -> Quat {
        attitude(self.yaw, self.push)
    }

    /// Its pose to send, at `time` on the server's clock.
    pub fn pose(&self, time: f64) -> HeliPose {
        HeliPose {
            time,
            position: self.position,
            velocity: self.velocity,
            rotation: self.rotation(),
            cable: 0.0,
            hooked: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target driving along x as `speed(t)` says, the chase after it for `seconds`:
    /// the farthest the chase was ahead of it, and where both were at the end.
    fn chase(seconds: f32, speed: impl Fn(f32) -> f32) -> (f32, Chase, Vec3) {
        let dt = 1.0 / 60.0;
        let mut target = Vec3::ZERO;
        let mut heli = Chase::new(target, 0.0);
        let mut ahead = f32::NEG_INFINITY;
        let mut t = 0.0;
        while t < seconds {
            let v = Vec3::X * speed(t);
            target += v * dt;
            heli.step(target, v, 0.0, dt);
            ahead = ahead.max(heli.position.x - target.x);
            t += dt;
        }
        (ahead, heli, target)
    }

    #[test]
    fn the_chase_cannot_be_outrun() {
        // Flat out at 300 km/h for two minutes: it's right above at the end.
        let (_, heli, target) = chase(120.0, |_| 300.0 / 3.6);
        let off = (heli.position - target).xz().length();
        assert!(off < 5.0, "{off} m behind");
        assert!((heli.position.y - CHASE_HEIGHT).abs() < 1.0);
        // Facing the way it flies, nose down.
        let forward = heli.rotation() * Vec3::NEG_Z;
        assert!(forward.x > 0.9 && forward.y < -0.1, "{forward}");
    }

    #[test]
    fn the_chase_overshoots_a_hard_stop_and_comes_back() {
        // 200 km/h, then braking at 10 m/s² to a stop and waiting.
        let stop = |t: f32| (200.0 / 3.6 - 10.0 * (t - 30.0).max(0.0)).max(0.0);
        let (ahead, heli, target) = chase(80.0, stop);
        assert!(ahead > 30.0, "only {ahead} m past");
        let off = (heli.position - target).xz().length();
        assert!(off < CHASE_WANDER + 3.0, "{off} m off at the end");
        // It came back: it last flew towards −x, facing that way at some point.
        assert!(heli.velocity.length() < 3.0, "{}", heli.velocity);
    }

    #[test]
    fn the_chase_keeps_off_the_ground() {
        let mut heli = Chase::new(Vec3::ZERO, 0.0);
        for _ in 0..1200 {
            heli.step(Vec3::ZERO, Vec3::ZERO, 40.0, 1.0 / 60.0);
        }
        assert!(
            heli.position.y > 40.0 + CHASE_CLEARANCE - 1.0,
            "{}",
            heli.position.y
        );
    }

    #[test]
    fn attitude_leans_into_the_push() {
        let level = attitude(0.0, Vec3::ZERO);
        assert!((level * Vec3::Y).y > 0.999);
        // Pushing forward (−z) tips the nose down.
        let forward = attitude(0.0, Vec3::NEG_Z * 4.0) * Vec3::NEG_Z;
        assert!(forward.y < -0.2, "{forward}");
        // Never past the most tilt.
        let hard = attitude(0.0, Vec3::X * 100.0) * Vec3::Y;
        assert!(hard.angle_between(Vec3::Y) <= MOST_TILT + 1e-4);
    }
}
