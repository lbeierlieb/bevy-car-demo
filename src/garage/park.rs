//! The garage park: everyone their own garage there. Its window lists the cars in the
//! garage (and the trunk and parts, as every shop has); "Into the garage" goes into the
//! player's own: a hall deep under the marker, the car driven in the first bay and the
//! cars kept there in the bays beside it, the parts kept there on shelves along the back
//! wall. The window in it drives another car (the camera looks at the one a row is
//! about) and opens the trunk and parts.
//!
//! Like the shops' scenes, the driven car stays parked on the marker meanwhile (a ghost,
//! its input held), and everything in the hall is drawn from looks. The hall is built
//! again whenever what's in it changes (another car driven, a part unloaded).

use std::f32::consts::TAU;
use std::hash::{Hash, Hasher};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::audio::EngineAudio;
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car, CarSetup};
use crate::catalogue::{Catalogue, Current};
use crate::items::{Does, Item};
use crate::looks::{Looks, Motion};
use crate::menu::kit::{self, Panel, Place};
use crate::net::Connection;
use crate::net::protocol::ToServer;
use crate::owned::Stash;
use crate::shops::{AtShop, SceneBack, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;
use crate::trunk::{Screen, car_name};

/// How far under the ground at the garage park the hall is (m).
const DEPTH: f32 = 250.0;
/// A bay's width, the hall's half length (along the cars) and height (m).
const BAY: f32 = 3.8;
/// The fewest bays a hall has (some empty), for room to look round.
const LEAST_BAYS: usize = 3;
const HALF_LENGTH: f32 = 4.6;
const HEIGHT: f32 = 3.6;
/// The shelves along the back wall: how deep, the heights of their boards (m), and how many
/// parts are shown on them at most.
const SHELF_DEPTH: f32 = 0.7;
const BOARDS: [f32; 3] = [0.08, 0.95, 1.8];
const MOST_SHOWN: usize = 120;
/// The camera: from where it looks at a car (in front of it and to its left), how near and
/// far it may zoom, and how far inside the walls it keeps (m).
const EYE: Vec3 = Vec3::new(-3.2, 1.7, -4.4);
const LOOK_UP: f32 = 0.75;
const ZOOM: std::ops::RangeInclusive<f32> = 0.5..=1.4;
const WALL_GAP: f32 = 0.3;

pub struct ParkPlugin;

impl Plugin for ParkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hall>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (rebuild, look.after(crate::camera::follow)).run_if(in_hall),
                )
                    .chain(),
            )
            .add_systems(EguiPrimaryContextPass, (shop, window.run_if(in_hall)));
    }
}

/// The hall, while the player is in it: where it is, how many bays it has, what it was
/// built of (to build it again when that changes), the car looked at (0: the one driven,
/// then the garage's) and the camera's swing and zoom.
#[derive(Resource, Default)]
struct Hall {
    inside: bool,
    origin: Vec3,
    bays: usize,
    built: Option<u64>,
    looked_at: usize,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
}

fn in_hall(hall: Res<Hall>) -> bool {
    hall.inside
}

/// Everything in the hall, which goes when the player leaves it (or it's built again).
#[derive(Component)]
struct InHall;

/// Where bay `bay` is across the hall (its middle, m).
fn bay_x(bay: usize) -> f32 {
    bay as f32 * BAY
}

/// The garage park's window: the cars in the garage, into the garage, and the trunk and
/// parts as at every shop.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    connection: Option<Res<Connection>>,
    mut hall: ResMut<Hall>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
) -> Result {
    if hall.inside {
        return Ok(());
    }
    let cars = connection.as_ref().map_or(0, |c| c.owned.cars.len());
    let parts = connection.as_ref().map_or(0, |c| c.owned.parts.len());
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let (mut enter, mut go) = (false, None);
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Garage, |ui, _| {
        enter = kit::button(ui, "Into your garage").clicked();
        if kit::submenu_with(ui, "Your cars", &format!("{} more", cars)).clicked() {
            go = Some(Screen::Cars);
        }
        kit::note(
            ui,
            &format!(
                "Your own garage, for the cars and parts you aren't using: {parts} parts in it."
            ),
        );
    });
    if let Some(screen) = go {
        at.stash.push(screen);
    }
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    *hall = Hall {
        inside: true,
        origin: Vec3::new(marker.x, ground - DEPTH, marker.y),
        zoom: 1.0,
        ..default()
    };
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.9, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    Ok(())
}

/// Out of the hall and back to the driven car, parked at the garage park.
fn leave(
    commands: &mut Commands,
    hall: &mut Hall,
    things: &Query<Entity, With<InHall>>,
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
    *hall = Hall::default();
    away.0 = false;
    audio.hushed = false;
}

/// The car left the garage park (or the server, and with it the garage) while in the hall:
/// out.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut hall: ResMut<Hall>,
    things: Query<Entity, With<InHall>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if hall.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut hall, &things, parts, &mut cameras);
    }
}

/// A car standing still: its wheels at their design height, straight, the engine off.
fn at_rest(looks: &Looks) -> Motion {
    let mut motion = Motion::default();
    for (corner, wheel) in motion.wheels.iter_mut().enumerate() {
        wheel.theta = looks.axles[corner / 2].geometry.design_angle();
    }
    motion
}

/// What the hall is built of, as a number: the car driven and what's kept in the garage.
fn key(car: &Item, owned: &crate::owned::OwnedView) -> u64 {
    use crate::net::wire::encode;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    encode(car).hash(&mut hasher);
    encode(owned).hash(&mut hasher);
    hasher.finish()
}

/// Builds the hall (again) when what's in it changed.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut hall: ResMut<Hall>,
    things: Query<Entity, With<InHall>>,
    connection: Option<Res<Connection>>,
    catalogue: Option<Res<Current>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) {
    let (Some(connection), Some(catalogue)) = (connection, catalogue) else {
        return;
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let owned = &connection.owned;
    let key = key(&car.setup.car, owned);
    if hall.built == Some(key) {
        return;
    }
    for entity in &things {
        commands.entity(entity).despawn();
    }
    hall.built = Some(key);
    hall.bays = (owned.cars.len() + 1).max(LEAST_BAYS);
    hall.looked_at = hall.looked_at.min(owned.cars.len());
    let catalogue = &catalogue.0;
    let mut looks = vec![Looks::of(&car.setup)];
    looks.extend(
        (owned.cars.iter())
            .filter_map(|parked| CarSetup::looked_at(catalogue, &parked.car, &parked.cargo))
            .map(|setup| Looks::of(&setup)),
    );
    build_hall(
        &mut commands,
        &hall,
        &looks,
        &owned.parts,
        catalogue,
        &mut meshes,
        &mut materials,
    );
}

/// The hall: concrete floor with yellow lines between the bays, grey walls, a roller door
/// ahead of each bay, light strips and lamps in the ceiling, shelves along the back wall
/// with the parts on them; a car in each bay.
fn build_hall(
    commands: &mut Commands,
    hall: &Hall,
    looks: &[Looks],
    parts: &[Item],
    catalogue: &Catalogue,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let lamp = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(6.0, 6.0, 5.6),
        unlit: true,
        ..default()
    });
    let mut material = |color: Color, roughness: f32, metallic: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic,
            ..default()
        })
    };
    let floor = material(Color::srgb(0.47, 0.47, 0.46), 0.8, 0.0);
    let line = material(Color::srgb(0.85, 0.7, 0.1), 0.6, 0.0);
    let wall = material(Color::srgb(0.72, 0.73, 0.74), 0.8, 0.0);
    let band = material(Color::srgb(0.25, 0.27, 0.3), 0.7, 0.0);
    let door = material(Color::srgb(0.6, 0.62, 0.64), 0.45, 0.6);
    let rib = material(Color::srgb(0.5, 0.52, 0.54), 0.4, 0.7);
    let steel = material(Color::srgb(0.3, 0.42, 0.62), 0.5, 0.6);
    let board = material(Color::srgb(0.55, 0.45, 0.3), 0.8, 0.0);
    let cardboard = material(Color::srgb(0.66, 0.52, 0.34), 0.95, 0.0);
    let metal = material(Color::srgb(0.55, 0.56, 0.58), 0.35, 0.85);
    let rubber = material(Color::srgb(0.06, 0.06, 0.07), 0.9, 0.0);
    let paints: Vec<Handle<StandardMaterial>> = (catalogue.paints.iter())
        .map(|paint| material(Color::srgb_from_array(paint.color), 0.35, 0.2))
        .collect();
    let mut lights = Vec::new();
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let origin = hall.origin;
    let mut block = |at: Vec3, size: Vec3, material: &Handle<StandardMaterial>| {
        commands.spawn((
            InHall,
            Mesh3d(cube.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(origin + at).with_scale(size),
        ));
    };
    let left = -BAY / 2.0 - 0.6;
    let right = bay_x(hall.bays - 1) + BAY / 2.0 + 0.6;
    let width = right - left;
    let middle = (left + right) / 2.0;
    let l = HALF_LENGTH + SHELF_DEPTH;
    // Floor, ceiling, walls (a dark band along their foot).
    block(
        Vec3::new(middle, -0.05, 0.0),
        Vec3::new(width, 0.1, 2.0 * l),
        &floor,
    );
    block(
        Vec3::new(middle, HEIGHT + 0.05, 0.0),
        Vec3::new(width, 0.1, 2.0 * l),
        &wall,
    );
    for (x, w) in [(left, 0.1), (right, 0.1)] {
        block(
            Vec3::new(x, HEIGHT / 2.0, 0.0),
            Vec3::new(w, HEIGHT, 2.0 * l),
            &wall,
        );
        block(
            Vec3::new(x, 0.15, 0.0),
            Vec3::new(w + 0.02, 0.3, 2.0 * l),
            &band,
        );
    }
    block(
        Vec3::new(middle, HEIGHT / 2.0, l),
        Vec3::new(width, HEIGHT, 0.1),
        &wall,
    );
    block(
        Vec3::new(middle, HEIGHT / 2.0, -l),
        Vec3::new(width, HEIGHT, 0.1),
        &wall,
    );
    for bay in 0..hall.bays {
        let x = bay_x(bay);
        // A roller door ahead of each bay, its ribs across.
        let door_width = BAY - 0.6;
        block(
            Vec3::new(x, 1.3, -l + 0.07),
            Vec3::new(door_width, 2.6, 0.04),
            &door,
        );
        for k in 0..12 {
            let y = 0.1 + k as f32 * 0.21;
            block(
                Vec3::new(x, y, -l + 0.1),
                Vec3::new(door_width, 0.03, 0.02),
                &rib,
            );
        }
        // Lines between the bays, a light strip over each, a lamp lighting it.
        if bay > 0 {
            block(
                Vec3::new(x - BAY / 2.0, 0.003, -0.3),
                Vec3::new(0.1, 0.006, 2.0 * HALF_LENGTH - 1.0),
                &line,
            );
        }
        block(
            Vec3::new(x, HEIGHT - 0.03, 0.0),
            Vec3::new(0.2, 0.04, 3.0),
            &lamp,
        );
        lights.push(origin + Vec3::new(x, HEIGHT - 0.5, 0.0));
    }
    // The shelves along the back wall: uprights and boards.
    let shelf_z = l - SHELF_DEPTH / 2.0 - 0.05;
    let uprights = ((width / 2.0).ceil() as usize).max(1);
    for k in 0..=uprights {
        let x = left + 0.1 + (width - 0.2) * k as f32 / uprights as f32;
        for dz in [-SHELF_DEPTH / 2.0, SHELF_DEPTH / 2.0] {
            block(
                Vec3::new(x, 1.25, shelf_z + dz),
                Vec3::new(0.06, 2.5, 0.06),
                &steel,
            );
        }
    }
    for y in BOARDS {
        block(
            Vec3::new(middle, y - 0.02, shelf_z),
            Vec3::new(width - 0.2, 0.04, SHELF_DEPTH),
            &board,
        );
    }
    // The parts on them, left to right, board after board.
    let (mut x, mut level) = (left + 0.2, 0);
    for item in parts.iter().take(MOST_SHOWN) {
        let def = item.def(catalogue);
        // Lying flat, its longest side along the shelf.
        let mut size = def.size;
        size.sort_by(|a, b| b.total_cmp(a));
        let size = Vec3::new(size[0], size[2], size[1].min(SHELF_DEPTH - 0.05));
        if x + size.x > right - 0.2 {
            (x, level) = (left + 0.2, level + 1);
        }
        let Some(&y) = BOARDS.get(level) else {
            break;
        };
        let look = match def.does {
            Does::Wheel | Does::Tire(_) => &rubber,
            Does::Rim | Does::Block { .. } | Does::Gearbox(_) | Does::Brake(_) => &metal,
            _ => item.paint.and_then(|p| paints.get(p)).unwrap_or(&cardboard),
        };
        block(
            Vec3::new(x + size.x / 2.0, y + size.y / 2.0, shelf_z),
            size,
            look,
        );
        x += size.x + 0.08;
    }
    for at in lights {
        commands.spawn((
            InHall,
            PointLight {
                intensity: 600_000.0,
                range: 14.0,
                ..default()
            },
            Transform::from_translation(at),
        ));
    }
    // A car in each bay, facing the doors.
    for (bay, looks) in looks.iter().enumerate() {
        let model = Transform::from_translation(
            origin + Vec3::new(bay_x(bay), 0.0, -0.3) - looks.body_offset,
        );
        commands.spawn((InHall, looks.clone(), at_rest(looks), model));
    }
}

/// The camera looks at the car a row is about, from in front of it and to its left; drag
/// swings it round, the wheel zooms.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut hall: ResMut<Hall>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        hall.swing = (hall.swing - motion.delta.x * 0.005).rem_euclid(TAU);
    }
    let zoom = hall.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    hall.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = Vec3::new(bay_x(hall.looked_at), LOOK_UP, -0.3);
    let eye = target + Quat::from_rotation_y(hall.swing) * EYE * hall.zoom;
    let (left, right) = (
        -BAY / 2.0 - 0.6,
        bay_x(hall.bays.max(1) - 1) + BAY / 2.0 + 0.6,
    );
    let l = HALF_LENGTH + SHELF_DEPTH - WALL_GAP;
    let eye = eye.clamp(
        Vec3::new(left + WALL_GAP, 0.3, -l),
        Vec3::new(right - WALL_GAP, HEIGHT - WALL_GAP, l - SHELF_DEPTH),
    );
    let (eye, target) = (hall.origin + eye, hall.origin + target);
    let ease = 1.0 - (-5.0 * time.delta_secs()).exp();
    let now = hall.eye.map_or(eye, |was| was.lerp(eye, ease));
    hall.eye = Some(now);
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(target, Vec3::Y);
    }
}

/// The window in the hall: back to the car, the cars (each driven at a click; the camera
/// looks at the one a row is about), and the trunk and parts.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut hall: ResMut<Hall>,
    things: Query<Entity, With<InHall>>,
    mut at: ResMut<AtShop>,
    (connection, catalogue): (Option<ResMut<Connection>>, Option<Res<Current>>),
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    (mut cameras, mut back): (Query<(&ChaseCamera, &mut Projection)>, ResMut<SceneBack>),
) -> Result {
    let out = back.take();
    if !at.stash.is_empty() {
        // The trunk and parts screens are up (`trunk`): back goes back a screen there.
        if out {
            at.stash.pop();
        }
        return Ok(());
    }
    let (Some(mut connection), Some(catalogue)) = (connection, catalogue) else {
        return Ok(());
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let catalogue = &catalogue.0;
    let owned = &connection.owned;
    let (mut leave_it, mut drive, mut stash) = (out, None, false);
    let mut looked_at = None;
    let ctx = contexts.ctx_mut()?;
    Panel::new("garage hall", "Your garage")
        .place(Place::Right)
        .width(420.0)
        .back(Some("Back to the car"))
        .show(ctx, |ui| {
            if kit::button(ui, "Back to the car").clicked() {
                leave_it = true;
            }
            kit::section(ui, "Cars");
            let driven = kit::button_with(ui, &car_name(catalogue, &car.setup.car), "You drive it");
            if kit::focused(ui, &driven) || driven.hovered() {
                looked_at = Some(0);
            }
            for (i, parked) in owned.cars.iter().enumerate() {
                let row = kit::button_with(ui, &car_name(catalogue, &parked.car), "Drive it");
                if kit::focused(ui, &row) || row.hovered() {
                    looked_at = Some(i + 1);
                }
                if row.clicked() {
                    drive = Some(i);
                }
            }
            kit::note(
                ui,
                "Driving another, the one you drive takes its place here.",
            );
            kit::section(ui, "Parts");
            let summary = format!(
                "{} here, {} in the trunk",
                owned.parts.len(),
                car.setup.cargo.len()
            );
            if kit::submenu_with(ui, Screen::List.title(), &summary).clicked() {
                stash = true;
            }
        });
    if let Some(i) = looked_at {
        hall.looked_at = i;
    }
    if let Some(i) = drive {
        connection.refused = None;
        connection.say(&ToServer::Stash(Stash::Drive { car: i as u16 }));
        hall.looked_at = 0;
    }
    if stash {
        at.stash.push(Screen::List);
    }
    if leave_it {
        leave(
            &mut commands,
            &mut hall,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}
