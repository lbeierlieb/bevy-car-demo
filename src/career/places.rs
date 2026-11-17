//! The career's places: a house, shop or yard each, with a glowing marker on the road in
//! front of it. Stopping on a marker is being at the place (`AtPlace`), which opens its
//! window (`menus`). Their names float above them when near, and they're on the map.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::{Scene, driven};
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Car};
use crate::map::{MapMark, MapMarks};
use crate::terrain::Terrain;

/// Within this distance of a marker's middle a car is on it (m), and it has to stop
/// (m/s); once at a place, it's left by moving this far away.
pub const MARKER_RADIUS: f32 = 3.5;
pub const STOPPED: f32 = 1.5;
const LEAVE: f32 = 6.0;
/// Names float above markers nearer than this (m).
const LABEL_RANGE: f32 = 500.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaceKind {
    Home,
    GasStation,
    Pizzeria,
    Dealer,
    PartsShop,
}

impl PlaceKind {
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Home => [90, 200, 255],
            Self::GasStation => [255, 200, 40],
            Self::Pizzeria => [255, 90, 60],
            Self::Dealer => [80, 230, 120],
            Self::PartsShop => [200, 120, 255],
        }
    }
}

/// The places, by the middle of the building (map x and z, from OSM).
const PLACES: [(PlaceKind, &str, [f32; 2]); 5] = [
    (PlaceKind::Home, "Home", [-2996.0, -2013.0]),
    (PlaceKind::GasStation, "Gas station", [-2603.0, -1736.0]),
    (
        PlaceKind::Pizzeria,
        "Pizzeria Bella Strada",
        [-1129.0, -908.0],
    ),
    (PlaceKind::Dealer, "Kessler Cars", [-1476.0, -1213.0]),
    (
        PlaceKind::PartsShop,
        "Motor Parts Thiel",
        [-1950.0, -3160.0],
    ),
];

pub struct Place {
    pub kind: PlaceKind,
    pub name: &'static str,
    /// The marker: on the road in front of the building, at the edge on its side.
    pub at: Vec2,
    /// Facing along the road there (yaw), which way a car leaving the place is put down.
    pub yaw: f32,
}

#[derive(Resource)]
pub struct Places {
    pub list: Vec<Place>,
}

impl Places {
    fn new(terrain: &Terrain) -> Self {
        let list = PLACES
            .iter()
            .map(|&(kind, name, [x, z])| {
                let (at, yaw) = kerb(terrain, Vec2::new(x, z));
                Place {
                    kind,
                    name,
                    at,
                    yaw,
                }
            })
            .collect();
        Self { list }
    }

    pub fn of(&self, kind: PlaceKind) -> &Place {
        self.list
            .iter()
            .find(|place| place.kind == kind)
            .expect("every kind of place is on the map")
    }
}

/// Where a car stops for the building at `p`: on the nearest paved road, at the edge on the
/// building's side; and the yaw along the road there.
pub fn kerb(terrain: &Terrain, p: Vec2) -> (Vec2, f32) {
    let Some((point, along, width)) = terrain.paved_road_near(p) else {
        return (p, 0.0);
    };
    let side = (p - point).normalize_or_zero();
    let at = point + side * (width / 2.0 - 1.5).max(0.0);
    (at, (-along.x).atan2(-along.y))
}

/// The place the car being driven is stopped at, if any.
#[derive(Resource, Default)]
pub struct AtPlace(pub Option<usize>);

/// A glowing disc on the road and a beam above it.
#[derive(Component)]
pub struct Marker;

pub struct PlacesPlugin;

impl Plugin for PlacesPlugin {
    fn build(&self, app: &mut App) {
        let terrain = app
            .world()
            .get_resource::<Terrain>()
            .expect("the terrain is loaded before the career");
        let places = Places::new(terrain);
        app.insert_resource(places)
            .init_resource::<AtPlace>()
            .add_systems(Startup, (spawn_markers, mark_map))
            .add_systems(Update, arrive.run_if(in_state(Scene::Driving)))
            .add_systems(
                EguiPrimaryContextPass,
                labels.run_if(in_state(Scene::Driving)),
            );
    }
}

/// A marker of `color` on the ground at `at`.
pub fn spawn_marker(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    terrain: &Terrain,
    at: Vec2,
    [r, g, b]: [u8; 3],
) -> Entity {
    let color = Color::srgb_u8(r, g, b);
    let mut glow = |alpha: f32| {
        materials.add(StandardMaterial {
            base_color: color.with_alpha(alpha),
            emissive: color.to_linear() * 2.0,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })
    };
    let (disc, beam) = (glow(0.45), glow(0.18));
    let ground = terrain.sample(at.x, at.y).0;
    commands
        .spawn((
            Marker,
            Transform::from_xyz(at.x, ground, at.y),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Mesh3d(meshes.add(Cylinder::new(MARKER_RADIUS, 0.05))),
                MeshMaterial3d(disc),
                Transform::from_xyz(0.0, 0.12, 0.0),
                NotShadowCaster,
            ));
            parent.spawn((
                Mesh3d(meshes.add(Cylinder::new(0.35, 40.0))),
                MeshMaterial3d(beam),
                Transform::from_xyz(0.0, 20.0, 0.0),
                NotShadowCaster,
            ));
        })
        .id()
}

fn spawn_markers(
    mut commands: Commands,
    places: Res<Places>,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for place in &places.list {
        let color = place.kind.color();
        spawn_marker(
            &mut commands,
            &mut meshes,
            &mut materials,
            &terrain,
            place.at,
            color,
        );
    }
}

fn mark_map(places: Res<Places>, mut marks: ResMut<MapMarks>) {
    marks.0.extend(places.list.iter().map(|place| MapMark {
        at: place.at,
        color: place.kind.color(),
        label: place.name.to_string(),
    }));
}

/// Stopping on a marker is arriving at its place; moving off it, leaving.
fn arrive(active: Res<ActiveCar>, cars: Query<&Car>, places: Res<Places>, mut at: ResMut<AtPlace>) {
    let Some(car) = driven(&cars, &active) else {
        at.0 = None;
        return;
    };
    let p = car.body.position.xz();
    match at.0 {
        Some(i) if places.list[i].at.distance(p) > LEAVE => at.0 = None,
        None if car.body.velocity.length() < STOPPED => {
            at.0 = (places.list.iter()).position(|place| place.at.distance(p) < MARKER_RADIUS);
        }
        _ => {}
    }
}

/// The places' names above their markers, when they're near and in view.
fn labels(
    mut contexts: EguiContexts,
    places: Res<Places>,
    terrain: Res<Terrain>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let Ok((camera, view)) = cameras.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    for place in &places.list {
        let ground = terrain.sample(place.at.x, place.at.y).0;
        let top = Vec3::new(place.at.x, ground + 6.0, place.at.y);
        let distance = top.distance(view.translation());
        if distance > LABEL_RANGE {
            continue;
        }
        let Ok(screen) = camera.world_to_viewport(view, top) else {
            continue;
        };
        let at = egui::pos2(screen.x, screen.y);
        let [r, g, b] = place.kind.color();
        let text = format!("{}  {:.0} m", place.name, distance);
        let galley = painter.layout_no_wrap(text, FontId::proportional(14.0), Color32::WHITE);
        let rect = Align2::CENTER_BOTTOM
            .anchor_size(at, galley.size())
            .expand(5.0);
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(170));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.5, Color32::from_rgb(r, g, b)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + egui::vec2(5.0, 5.0), galley, Color32::WHITE);
    }
    Ok(())
}
