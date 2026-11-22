//! The paint shops: the car in a spray booth of their own (white walls, a lit filter
//! ceiling, floor grates along the car, the doors ahead, filters at the back), a shelf of
//! tins and a test card on a stand in the picked colour. The window on the right picks a
//! paint and sprays it: the booth fills with a mist of the new colour, which the downdraft
//! clears from the car in it.
//!
//! The booth is under the shop (like the alignment bay); the driven car stays parked on the
//! shop's marker meanwhile (a ghost, its input held like in the dyno's room), and the car in
//! the booth is drawn from its looks, the engine off. The paint goes onto the driven car
//! when the mist is thickest.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::audio::{EngineAudio, EngineRun};
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car};
use crate::chassis::{PAINTS, SHOP_PAINTS};
use crate::looks::{Looks, Motion};
use crate::shops::{self, AtShop, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;

/// How far under the ground at the shop the booth is (m).
const DEPTH: f32 = 240.0;
/// The booth inside: at least this wide and long (m), this much room round the car's body,
/// and this high.
const LEAST: Vec2 = Vec2::new(5.6, 9.0);
const ROOM_AROUND: Vec2 = Vec2::new(1.9, 2.2);
const HEIGHT: f32 = 3.4;
/// How long a coat takes (s), and when the mist is thickest (and the paint goes on).
const SPRAY: f32 = 6.0;
const THICKEST: f32 = 3.0;
/// The mist at its thickest: shells round the car (each as opaque as this, seen through
/// twice), from this far out round the body to that far (m), and droplets swirling in them.
const SHELLS: usize = 12;
const SHELL: f32 = 0.085;
const SHELL_GAP: std::ops::RangeInclusive<f32> = 0.1..=0.95;
const DROPLETS: usize = 600;
/// Droplets sink from this high to the floor (m).
const DROP_SPAN: f32 = 2.2;
/// The camera: where it looks (over the car's middle) and from where, and how near and far
/// it may zoom; it keeps this far inside the walls (m).
const EYE: Vec3 = Vec3::new(-3.4, 1.55, -4.2);
const LOOK_UP: f32 = 0.7;
const ZOOM: std::ops::RangeInclusive<f32> = 0.5..=1.3;
const WALL_GAP: f32 = 0.25;

pub struct PaintPlugin;

impl Plugin for PaintPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Booth>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (spray, dress, mist, look.after(crate::camera::follow)).run_if(in_booth),
                )
                    .chain(),
            )
            .add_systems(EguiPrimaryContextPass, (shop, window.run_if(in_booth)));
    }
}

/// The booth, while the car is in it: where it is, its size, the camera's swing and zoom,
/// the paint picked and the coat being sprayed.
#[derive(Resource, Default)]
struct Booth {
    inside: bool,
    origin: Vec3,
    /// Half the width, the height and half the length inside (m).
    room: Vec3,
    /// Where the camera looks (in the booth's frame), its swing and zoom.
    target: Vec3,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
    /// The paint picked in the window (in `PAINTS`).
    picked: usize,
    spray: Option<Spray>,
}

/// A coat being sprayed: the paint, and how long it's been going (s).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spray {
    paint: usize,
    time: f32,
}

fn in_booth(booth: Res<Booth>) -> bool {
    booth.inside
}

/// Everything in the booth, which goes when the car leaves it.
#[derive(Component)]
struct InBooth;

/// The car in the booth, drawn from the driven car's `Looks`.
#[derive(Component)]
struct CarModel;

/// A tin of a paint on the shelf, where it stands (the picked one is pulled forward).
#[derive(Component)]
struct Tin {
    paint: usize,
    at: Vec3,
}

/// The mist: a shell round the car (its middle in the world and its half sizes), or a
/// droplet (going round the car's middle at a radius and height, from an angle).
#[derive(Component, Clone, Copy)]
enum Mist {
    Shell {
        at: Vec3,
        half: Vec3,
    },
    Droplet {
        middle: Vec3,
        radius: Vec2,
        height: f32,
        angle: f32,
    },
}

/// The materials that change: the test card (the picked paint), and the mist's shells and
/// droplets (the paint being sprayed, as thick as it is).
#[derive(Resource)]
struct BoothPaints {
    card: Handle<StandardMaterial>,
    shells: Handle<StandardMaterial>,
    droplets: Handle<StandardMaterial>,
}

/// How thick the mist is (0..1) `time` into a coat: rising to its thickest, then drawn
/// down by the downdraft.
fn thickness(time: f32) -> f32 {
    let smooth = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    if time < THICKEST {
        smooth(time / THICKEST)
    } else {
        1.0 - smooth((time - THICKEST - 0.3) / (SPRAY - THICKEST - 0.3))
    }
}

/// Sprays on for `dt`: the paint goes onto `car` when the mist is thickest; the coat is
/// done when it has cleared.
fn spray_coat(spray: &mut Option<Spray>, car: &mut Car, dt: f32) {
    let Some(coat) = spray else {
        return;
    };
    let was = coat.time;
    coat.time += dt;
    if was < THICKEST && coat.time >= THICKEST {
        let mut build = car.setup.build.clone();
        build.paint = coat.paint;
        shops::rebuild(car, build);
    }
    if coat.time >= SPRAY {
        *spray = None;
    }
}

/// The paint shop's window (before the booth): the paint now, and into the booth.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut booth: ResMut<Booth>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if booth.inside {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Paint, |ui, _| {
        ui.label(format!("Now: {}", PAINTS[car.setup.build.paint].0));
        enter = ui
            .button(RichText::new("Into the paint booth").strong())
            .clicked();
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    let looks = Looks::of(&car.setup);
    let hull = hull_in_booth(&looks);
    let room = Vec3::new(
        (hull.half.x + ROOM_AROUND.x).max(LEAST.x / 2.0),
        HEIGHT,
        (hull.half.z + ROOM_AROUND.y).max(LEAST.y / 2.0),
    );
    *booth = Booth {
        inside: true,
        origin: Vec3::new(marker.x, ground - DEPTH, marker.y),
        room,
        target: Vec3::new(0.0, LOOK_UP, hull.center.z),
        zoom: 1.0,
        // The paint now, if it's one of the shop's (not a dull factory one).
        picked: if car.setup.build.paint < SHOP_PAINTS {
            car.setup.build.paint
        } else {
            0
        },
        ..default()
    };
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.9, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    build_booth(&mut commands, &booth, car, &mut meshes, &mut materials);
    Ok(())
}

/// The car's body as a box in the booth's frame (the body's design frame: on the floor
/// midway between the axles, facing -z): its middle and half sizes.
fn hull_in_booth(looks: &Looks) -> crate::physics::collision::Obb {
    let mut hull = looks.hull;
    hull.center -= looks.body_offset;
    hull
}

/// Out of the booth and back to the driven car, parked at the shop (painted already, if
/// the mist was thick by then).
fn leave(
    commands: &mut Commands,
    booth: &mut Booth,
    things: &Query<Entity, With<InBooth>>,
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
    (booth.inside, booth.spray) = (false, None);
    away.0 = false;
    audio.hushed = false;
}

/// The car left the shop (or the server, and with it the shop) while in the booth: out.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut booth: ResMut<Booth>,
    things: Query<Entity, With<InBooth>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if booth.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut booth, &things, parts, &mut cameras);
    }
}

/// Spawns the booth's boxes, cylinders and the like.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    origin: Vec3,
    cube: Handle<Mesh>,
}

impl Builder<'_, '_, '_> {
    /// A box of `size` with its middle at `at` (in the booth's frame).
    fn block(&mut self, at: Vec3, size: Vec3, material: &Handle<StandardMaterial>) -> Entity {
        let transform = Transform::from_translation(at).with_scale(size);
        self.put(&self.cube.clone(), material, transform)
    }

    /// `mesh` where `transform` (in the booth's frame) puts it.
    fn put(
        &mut self,
        mesh: &Handle<Mesh>,
        material: &Handle<StandardMaterial>,
        mut transform: Transform,
    ) -> Entity {
        transform.translation += self.origin;
        (self.commands)
            .spawn((
                InBooth,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ))
            .id()
    }
}

/// The booth round the car: floor with grates along both sides, white walls with panel
/// seams, a ceiling of lit filters and light rows in its edges, the doors ahead, a filter
/// bank at the back, the control panel and a door on the left, the shelf of tins and the
/// spray gun on the right, a test card on a stand; the car in the middle, and the mist.
fn build_booth(
    commands: &mut Commands,
    booth: &Booth,
    car: &Car,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mut material = |color: Color, roughness: f32, metallic: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic,
            ..default()
        })
    };
    let rgb = |[r, g, b]: [f32; 3]| Color::srgb(r, g, b);
    let floor = material(Color::srgb(0.42, 0.43, 0.44), 0.7, 0.0);
    let pit = material(Color::srgb(0.03, 0.03, 0.03), 1.0, 0.0);
    let grate = material(Color::srgb(0.55, 0.56, 0.58), 0.45, 0.8);
    let wall = material(Color::srgb(0.93, 0.94, 0.95), 0.55, 0.0);
    let seam = material(Color::srgb(0.70, 0.72, 0.74), 0.5, 0.2);
    let steel = material(Color::srgb(0.72, 0.73, 0.75), 0.3, 0.9);
    let frame = material(Color::srgb(0.30, 0.32, 0.35), 0.5, 0.4);
    let door = material(Color::srgb(0.82, 0.84, 0.86), 0.5, 0.1);
    let glass = material(Color::srgb(0.06, 0.08, 0.10), 0.1, 0.0);
    let filter = material(Color::srgb(0.95, 0.78, 0.30), 1.0, 0.0);
    let dark = material(Color::srgb(0.07, 0.07, 0.08), 0.5, 0.3);
    let hose = material(Color::srgb(0.10, 0.25, 0.55), 0.6, 0.0);
    let red = material(Color::srgb(0.8, 0.08, 0.06), 0.4, 0.0);
    let green = material(Color::srgb(0.1, 0.6, 0.2), 0.4, 0.0);
    let tin = material(Color::srgb(0.80, 0.81, 0.83), 0.35, 0.8);
    let lids: Vec<_> = PAINTS[..SHOP_PAINTS]
        .iter()
        .map(|(_, color)| material(rgb(*color), 0.3, 0.3))
        .collect();
    let card = material(rgb(PAINTS[booth.picked].1), 0.25, 0.4);
    let glow = |materials: &mut Assets<StandardMaterial>, color: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: Color::from(color),
            emissive: color,
            unlit: true,
            ..default()
        })
    };
    let lamp = glow(materials, LinearRgba::rgb(7.0, 7.0, 6.8));
    let ceiling = glow(materials, LinearRgba::rgb(1.5, 1.5, 1.48));
    let screen = glow(materials, LinearRgba::rgb(0.3, 1.6, 0.6));
    let mut haze = |cull_mode| {
        materials.add(StandardMaterial {
            base_color: rgb(PAINTS[booth.picked].1).with_alpha(0.0),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode,
            ..default()
        })
    };
    let (shells, droplets) = (
        haze(None),
        haze(Some(bevy::render::render_resource::Face::Back)),
    );
    commands.insert_resource(BoothPaints {
        card: card.clone(),
        shells: shells.clone(),
        droplets: droplets.clone(),
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut b = Builder {
        commands,
        origin: booth.origin,
        cube,
    };
    let looks = Looks::of(&car.setup);
    let hull = hull_in_booth(&looks);
    let Vec3 { x: w, y: h, z: l } = booth.room;

    // The floor: concrete down the middle, a grate over a pit along each side.
    b.block(
        Vec3::new(0.0, -0.05, 0.0),
        Vec3::new(2.0 * w + 0.4, 0.1, 2.0 * l + 0.4),
        &floor,
    );
    let inner = hull.half.x + 0.35;
    let grate_width = w - 0.25 - inner;
    for side in [-1.0, 1.0] {
        let x = side * (inner + grate_width / 2.0);
        let length = 2.0 * l - 1.0;
        b.block(
            Vec3::new(x, 0.002, 0.0),
            Vec3::new(grate_width, 0.004, length),
            &pit,
        );
        let bars = (grate_width / 0.06) as usize;
        for k in 0..=bars {
            let bx = x - grate_width / 2.0 + grate_width * k as f32 / bars as f32;
            b.block(
                Vec3::new(bx, 0.01, 0.0),
                Vec3::new(0.012, 0.016, length),
                &grate,
            );
        }
        for k in 0..=((length / 0.6) as usize) {
            let z = -length / 2.0 + 0.6 * k as f32;
            b.block(
                Vec3::new(x, 0.012, z),
                Vec3::new(grate_width, 0.018, 0.02),
                &grate,
            );
        }
    }

    // The walls, with seams between their panels.
    for side in [-1.0, 1.0] {
        b.block(
            Vec3::new(side * (w + 0.05), h / 2.0, 0.0),
            Vec3::new(0.1, h, 2.0 * l + 0.2),
            &wall,
        );
        b.block(
            Vec3::new(0.0, h / 2.0, side * (l + 0.05)),
            Vec3::new(2.0 * w + 0.2, h, 0.1),
            &wall,
        );
        let panels = (2.0 * l / 1.2).round() as usize;
        for k in 1..panels {
            let z = -l + 2.0 * l * k as f32 / panels as f32;
            b.block(
                Vec3::new(side * (w - 0.004), h / 2.0, z),
                Vec3::new(0.01, h, 0.025),
                &seam,
            );
        }
        // Kick plates along the bottom.
        b.block(
            Vec3::new(side * (w - 0.01), 0.15, 0.0),
            Vec3::new(0.02, 0.3, 2.0 * l),
            &steel,
        );
    }

    // The ceiling: filter panels glowing in a frame, light rows in its edges, angled at
    // the car.
    b.block(
        Vec3::new(0.0, h + 0.05, 0.0),
        Vec3::new(2.0 * w + 0.2, 0.1, 2.0 * l + 0.2),
        &frame,
    );
    let (across, along) = ((2.0 * w - 1.4) / 1.2, 2.0 * l / 1.2);
    let (across, along) = (across.round().max(1.0) as usize, along.round() as usize);
    let (tile_w, tile_l) = ((2.0 * w - 1.4) / across as f32, 2.0 * l / along as f32);
    for i in 0..across {
        for k in 0..along {
            let x = -w + 0.7 + tile_w * (i as f32 + 0.5);
            let z = -l + tile_l * (k as f32 + 0.5);
            b.block(
                Vec3::new(x, h - 0.01, z),
                Vec3::new(tile_w - 0.06, 0.01, tile_l - 0.06),
                &ceiling,
            );
        }
    }
    for side in [-1.0, 1.0] {
        let lean = Quat::from_rotation_z(side * FRAC_PI_4);
        let x = side * (w - 0.25);
        b.put(
            &b.cube.clone(),
            &frame,
            Transform::from_xyz(x, h - 0.25, 0.0)
                .with_rotation(lean)
                .with_scale(Vec3::new(0.66, 0.1, 2.0 * l - 0.6)),
        );
        let units = ((2.0 * l - 0.6) / 1.5) as usize;
        let unit = (2.0 * l - 0.6) / units as f32;
        for k in 0..units {
            let z = -l + 0.3 + unit * (k as f32 + 0.5);
            b.put(
                &b.cube.clone(),
                &lamp,
                Transform::from_xyz(x - side * 0.035, h - 0.285, z)
                    .with_rotation(lean)
                    .with_scale(Vec3::new(0.5, 0.02, unit - 0.12)),
            );
        }
        for z in [-l * 0.6, 0.0, l * 0.6] {
            b.commands.spawn((
                InBooth,
                PointLight {
                    intensity: 380_000.0,
                    range: 20.0,
                    ..default()
                },
                Transform::from_translation(booth.origin + Vec3::new(x * 0.7, h - 0.7, z)),
            ));
        }
    }

    // The doors ahead: three folding leaves with tall windows, in a frame.
    let leaf = (2.0 * w - 0.6) / 3.0;
    let door_h = h - 0.4;
    let face = -l + 0.01;
    b.block(
        Vec3::new(0.0, door_h + 0.06, face),
        Vec3::new(2.0 * w - 0.4, 0.12, 0.04),
        &frame,
    );
    for k in 0..3 {
        let x = -w + 0.3 + leaf * (k as f32 + 0.5);
        b.block(
            Vec3::new(x, door_h / 2.0, face),
            Vec3::new(leaf - 0.04, door_h, 0.03),
            &door,
        );
        b.block(
            Vec3::new(x, door_h * 0.6, face + 0.02),
            Vec3::new(leaf * 0.45, door_h * 0.55, 0.01),
            &glass,
        );
        b.block(
            Vec3::new(x + leaf * 0.38, 1.05, face + 0.04),
            Vec3::new(0.03, 0.3, 0.04),
            &steel,
        );
    }

    // The filter bank at the back, low down where the air goes out.
    let bank = 2.0 * w - 1.0;
    let pads = (bank / 0.6).round() as usize;
    let back = l - 0.01;
    b.block(
        Vec3::new(0.0, 0.75, back),
        Vec3::new(bank + 0.1, 1.3, 0.03),
        &frame,
    );
    for k in 0..pads {
        for row in 0..2 {
            let x = -bank / 2.0 + bank / pads as f32 * (k as f32 + 0.5);
            b.block(
                Vec3::new(x, 0.43 + 0.6 * row as f32, back - 0.02),
                Vec3::new(bank / pads as f32 - 0.05, 0.55, 0.02),
                &filter,
            );
        }
    }

    // On the left: the control panel near the doors, and a door at the back.
    let left = -w + 0.01;
    let panel = Vec3::new(left + 0.08, 1.45, -l + 1.2);
    b.block(panel, Vec3::new(0.14, 0.6, 0.45), &frame);
    b.block(
        panel + Vec3::new(0.075, 0.12, 0.0),
        Vec3::new(0.01, 0.2, 0.3),
        &screen,
    );
    for (k, light) in [&green, &red, &steel].into_iter().enumerate() {
        b.block(
            panel + Vec3::new(0.08, -0.15, -0.12 + 0.12 * k as f32),
            Vec3::new(0.03, 0.06, 0.06),
            light,
        );
    }
    let side_door = Vec3::new(left + 0.02, 1.0, l - 1.3);
    b.block(side_door, Vec3::new(0.03, 2.0, 0.95), &door);
    b.block(
        side_door + Vec3::new(0.02, 0.45, 0.0),
        Vec3::new(0.01, 0.4, 0.35),
        &glass,
    );
    b.block(
        side_door + Vec3::new(0.04, -0.05, -0.35),
        Vec3::new(0.04, 0.03, 0.15),
        &steel,
    );

    // On the right: the shelf of tins, the picked one pulled forward, and the spray gun on
    // its hook with the hose coiled under it.
    let right = w - 0.01;
    let shelf_z = l - 2.4;
    let shelf_len = 0.2 * SHOP_PAINTS.div_ceil(2) as f32 + 0.2;
    for y in [0.9, 1.4] {
        b.block(
            Vec3::new(right - 0.2, y, shelf_z),
            Vec3::new(0.4, 0.03, shelf_len),
            &steel,
        );
    }
    let can = meshes.add(Cylinder::new(0.075, 0.16));
    let lid = meshes.add(Cylinder::new(0.077, 0.012));
    for (paint, lid_paint) in lids.iter().enumerate() {
        let row = paint % 2;
        let z = shelf_z - shelf_len / 2.0 + 0.2 + 0.2 * (paint / 2) as f32;
        let at = Vec3::new(right - 0.22, [0.995, 1.495][row], z);
        let body = b.put(&can, &tin, Transform::from_translation(at));
        b.commands
            .entity(body)
            .insert(Tin { paint, at })
            .with_child((
                Mesh3d(lid.clone()),
                MeshMaterial3d(lid_paint.clone()),
                Transform::from_translation(Vec3::Y * 0.086),
            ));
    }
    let hook = Vec3::new(right - 0.05, 1.7, shelf_z + shelf_len / 2.0 + 0.6);
    b.block(hook, Vec3::new(0.1, 0.03, 0.03), &steel);
    // The gun: its cup on top, the body, the nozzle ahead and the grip down.
    let gun = hook + Vec3::new(-0.12, -0.08, 0.0);
    b.block(gun, Vec3::new(0.05, 0.06, 0.18), &steel);
    b.block(
        gun + Vec3::new(0.0, 0.0, -0.11),
        Vec3::new(0.025, 0.025, 0.05),
        &dark,
    );
    b.block(
        gun + Vec3::new(0.0, -0.1, 0.06),
        Vec3::new(0.035, 0.16, 0.04),
        &dark,
    );
    let cup = meshes.add(Cylinder::new(0.045, 0.12));
    b.put(
        &cup,
        &steel,
        Transform::from_translation(gun + Vec3::new(0.0, 0.09, -0.02)),
    );
    let coil = meshes.add(Torus::new(0.17, 0.19));
    for k in 0..3 {
        b.put(
            &coil,
            &hose,
            Transform::from_translation(hook + Vec3::new(-0.06, -0.55 - 0.02 * k as f32, 0.0))
                .with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        );
    }

    // The test card on a stand, by the car's right front wheel, turned to where the camera
    // starts.
    let stand = Vec3::new(w - 0.75, 0.0, hull.center.z - hull.half.z + 0.9);
    let to_eye = (EYE.xz() - stand.xz()).normalize_or(Vec2::Y);
    let turn = Quat::from_rotation_y(to_eye.x.atan2(to_eye.y));
    for k in 0..3 {
        let a = TAU * k as f32 / 3.0;
        let foot = stand + Vec3::new(a.cos(), 0.0, a.sin()) * 0.28;
        let top = stand + Vec3::Y * 1.05;
        b.put(
            &b.cube.clone(),
            &dark,
            Transform::from_translation((foot + top) / 2.0)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, (top - foot).normalize()))
                .with_scale(Vec3::new(0.02, foot.distance(top), 0.02)),
        );
    }
    let tilt = turn * Quat::from_rotation_x(-0.35);
    b.put(
        &b.cube.clone(),
        &frame,
        Transform::from_translation(stand + Vec3::Y * 1.25)
            .with_rotation(tilt)
            .with_scale(Vec3::new(0.46, 0.34, 0.012)),
    );
    b.put(
        &b.cube.clone(),
        &card,
        Transform::from_translation(stand + Vec3::Y * 1.25 + tilt * Vec3::Z * 0.008)
            .with_rotation(tilt)
            .with_scale(Vec3::new(0.42, 0.3, 0.006)),
    );

    // The car in the middle, the engine off.
    let (looks, motion) = looks_in_booth(car);
    let model = Transform::from_translation(booth.origin - looks.body_offset);
    b.commands.spawn((InBooth, CarModel, looks, motion, model));

    // The mist: shells round the car's body, one in another, and droplets in them.
    let middle = booth.origin + hull.center.with_y(hull.center.y.max(0.6));
    let shell = meshes.add(Sphere::new(1.0).mesh().uv(48, 24));
    for k in 0..SHELLS {
        let gap = SHELL_GAP.start()
            + (SHELL_GAP.end() - SHELL_GAP.start()) * k as f32 / (SHELLS - 1) as f32;
        // An ellipsoid through the box's corners is √3 its half sizes; a little less hugs
        // the body without its corners poking out much.
        let half = hull.half * 1.45 + Vec3::splat(gap);
        spawn_mist(&mut b, &shell, &shells, Mist::Shell { at: middle, half });
    }
    let droplet = meshes.add(Sphere::new(0.022).mesh().ico(1).expect("a small sphere"));
    for k in 0..DROPLETS {
        let mist = Mist::Droplet {
            middle: middle.with_y(booth.origin.y),
            radius: hull.half.xz() * (1.05 + 0.6 * scatter(k, 1)) + Vec2::splat(0.2),
            height: DROP_SPAN * scatter(k, 2),
            angle: TAU * scatter(k, 3),
        };
        spawn_mist(&mut b, &droplet, &droplets, mist);
    }
}

/// A number in 0..1 that looks random, the `salt`th for the `k`th droplet.
fn scatter(k: usize, salt: u32) -> f32 {
    let mut x = (k as u32).wrapping_mul(0x9e37_79b9) ^ salt.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1 << 24) as f32
}

/// A shell or droplet of the mist, hidden until a coat is sprayed.
fn spawn_mist(
    b: &mut Builder,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    mist: Mist,
) {
    b.commands.spawn((
        InBooth,
        mist,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        NotShadowCaster,
        Transform::default(),
        Visibility::Hidden,
    ));
}

/// The car in the booth: its looks and its suspension as the driven car's, the front
/// wheels straight, nothing turning, the engine off.
fn looks_in_booth(car: &Car) -> (Looks, Motion) {
    let mut motion = Motion::of(car);
    for wheel in &mut motion.wheels {
        (wheel.steer, wheel.spin) = (0.0, 0.0);
    }
    motion.engine = EngineRun::default();
    (Looks::of(&car.setup), motion)
}

fn spray(
    time: Res<Time>,
    mut booth: ResMut<Booth>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
) {
    if let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) {
        spray_coat(&mut booth.spray, &mut car, time.delta_secs());
    }
}

/// The car in the booth in the driven car's paint, the test card in the picked one, and
/// its tin pulled forward.
#[allow(clippy::type_complexity)]
fn dress(
    booth: Res<Booth>,
    paints: Option<Res<BoothPaints>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut models: Query<&mut Looks, With<CarModel>>,
    mut tins: Query<(&Tin, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if let Some(car) = cars.iter().find(|car| car.index == active.0) {
        for mut looks in &mut models {
            if looks.build.paint != car.setup.build.paint {
                *looks = Looks::of(&car.setup);
            }
        }
    }
    let color = Color::srgb_from_array(PAINTS[booth.picked].1);
    if let Some(mut card) = paints.and_then(|paints| materials.get_mut(&paints.card))
        && card.base_color != color
    {
        card.base_color = color;
    }
    for (tin, mut transform) in &mut tins {
        let out = if tin.paint == booth.picked {
            Vec3::new(-0.12, 0.0, 0.0)
        } else {
            Vec3::ZERO
        };
        let at = booth.origin + tin.at + out;
        if transform.translation != at {
            transform.translation = at;
        }
    }
}

/// The mist while a coat goes on: the paint's colour, as thick as it is, swirling and
/// sinking towards the grates.
fn mist(
    time: Res<Time>,
    booth: Res<Booth>,
    paints: Option<Res<BoothPaints>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut mists: Query<(&Mist, &mut Transform, &mut Visibility)>,
) {
    let Some(paints) = paints else {
        return;
    };
    let (thick, paint) = match booth.spray {
        Some(coat) => (thickness(coat.time), coat.paint),
        None => (0.0, booth.picked),
    };
    let color = Color::srgb_from_array(PAINTS[paint].1);
    for (handle, alpha) in [(&paints.shells, SHELL), (&paints.droplets, 0.6)] {
        if let Some(mut material) = materials.get_mut(handle) {
            let color = color.with_alpha(thick * alpha);
            if material.base_color != color {
                material.base_color = color;
            }
        }
    }
    let t = time.elapsed_secs();
    let shown = thick > 0.01;
    for (mist, mut transform, mut visibility) in &mut mists {
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }
        *transform = match *mist {
            // Shells swell as the mist thickens, and breathe a little.
            Mist::Shell { at, half } => {
                let breathe = 1.0 + 0.03 * (t * 1.3 + half.x).sin();
                Transform::from_translation(at).with_scale(half * (0.8 + 0.2 * thick) * breathe)
            }
            // Droplets go round the car and sink to the grates, coming back in at the top.
            Mist::Droplet {
                middle,
                radius,
                height,
                angle,
            } => {
                let a = angle + t * 0.5;
                let y = (height - t * 0.35).rem_euclid(DROP_SPAN);
                let at = Vec3::new(a.sin() * radius.x, y, a.cos() * radius.y);
                Transform::from_translation(middle + at)
            }
        };
    }
}

/// The camera round the car, inside the walls; dragging swings it round, the mouse wheel
/// takes it nearer or farther.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut booth: ResMut<Booth>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        booth.swing = (booth.swing - motion.delta.x * 0.005).rem_euclid(TAU);
    }
    let zoom = booth.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    booth.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = booth.target;
    let inside = booth.room - Vec3::new(WALL_GAP, WALL_GAP, WALL_GAP);
    let eye = target + Quat::from_rotation_y(booth.swing) * EYE * booth.zoom;
    let eye = eye.clamp(
        Vec3::new(-inside.x, 0.3, -inside.z),
        Vec3::new(inside.x, inside.y, inside.z),
    );
    let (eye, target) = (booth.origin + eye, booth.origin + target);
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    // Eased in the booth's frame along a circle would cut corners; a straight ease is
    // fine inside a box.
    let now = booth.eye.map_or(eye, |was| was.lerp(eye, ease));
    booth.eye = Some(now);
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(target, Vec3::Y);
    }
}

/// The booth's window, on the right: the paints as swatches, spraying the picked one; and
/// back to the car.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut booth: ResMut<Booth>,
    things: Query<Entity, With<InBooth>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) -> Result {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let now = car.setup.build.paint;
    let mut out = false;
    egui::Window::new("Paint booth")
        .anchor(Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(format!("Now: {}", PAINTS[now].0));
            let spraying = booth.spray;
            ui.add_enabled_ui(spraying.is_none(), |ui| {
                egui::Grid::new("paints").num_columns(5).show(ui, |ui| {
                    for (i, (name, [r, g, b])) in PAINTS[..SHOP_PAINTS].iter().enumerate() {
                        let color = Color32::from_rgb(
                            (r * 255.0) as u8,
                            (g * 255.0) as u8,
                            (b * 255.0) as u8,
                        );
                        let swatch = egui::Button::new("")
                            .fill(color)
                            .min_size(egui::vec2(48.0, 32.0))
                            .selected(i == booth.picked);
                        if ui.add(swatch).on_hover_text(*name).clicked() {
                            booth.picked = i;
                        }
                        if i % 5 == 4 {
                            ui.end_row();
                        }
                    }
                });
            });
            ui.label(format!("Picked: {}", PAINTS[booth.picked].0));
            match spraying {
                Some(coat) => {
                    let words = if coat.time < THICKEST {
                        "Spraying…"
                    } else {
                        "Clearing the air…"
                    };
                    ui.add(egui::ProgressBar::new(coat.time / SPRAY).text(words));
                }
                None => {
                    let picked = booth.picked;
                    let button = egui::Button::new(
                        RichText::new(format!("Spray it {}", PAINTS[picked].0)).strong(),
                    );
                    if ui
                        .add_enabled(picked != now, button)
                        .on_disabled_hover_text("The car is in this paint already")
                        .clicked()
                    {
                        booth.spray = Some(Spray {
                            paint: picked,
                            time: 0.0,
                        });
                    }
                }
            }
            ui.label(
                RichText::new("Drag to look round, the mouse wheel to go nearer.")
                    .small()
                    .weak(),
            );
            ui.separator();
            out = ui
                .button(RichText::new("Back to the car").strong())
                .clicked();
        });
    if out {
        leave(
            &mut commands,
            &mut booth,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;

    #[test]
    fn a_coat_goes_on_when_the_mist_is_thickest() {
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, 0.0, 0.0, 0.0);
        let paint = (car.setup.build.paint + 1) % SHOP_PAINTS;
        let mut spray = Some(Spray { paint, time: 0.0 });
        let dt = 0.1;
        let mut steps = 0;
        while spray.is_some() {
            let before = spray.unwrap().time;
            spray_coat(&mut spray, &mut car, dt);
            let painted = car.setup.build.paint == paint;
            assert_eq!(painted, before + dt >= THICKEST, "at {before} s");
            steps += 1;
        }
        assert!((steps as f32 * dt - SPRAY).abs() < 2.0 * dt);
        assert_eq!(car.setup.build.paint, paint);
        // The mist is thickest then, and gone at either end.
        assert!(thickness(0.0) < 1e-6 && thickness(SPRAY) < 1e-6);
        assert!((thickness(THICKEST) - 1.0).abs() < 1e-6);
    }
}
