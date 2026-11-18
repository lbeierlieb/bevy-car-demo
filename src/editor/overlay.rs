//! The map file drawn over the baked map near the camera (ways as lines draped on the
//! ground, coloured by what they are; point nodes as posts), and what's under the cursor.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;

use super::camera::FlyCamera;
use super::doc::Doc;
use super::index::{Class, Index, PointKind, baked_road};
use super::tools::{DrawPoint, Hover, Selection, Target, Tools};
use crate::map::MapView;
use crate::terrain::Terrain;

/// Lines float this far above the ground (m).
const LIFT: f32 = 0.3;
/// Draped lines get a point at least this often (m).
const DRAPE: f32 = 4.0;
/// How close to the cursor something must be to be picked (screen pixels).
const PICK_PIXELS: f32 = 12.0;
/// Point nodes are drawn as posts this tall (m).
const POST: f32 = 2.5;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Layers>()
            .init_resource::<Drapes>()
            .add_systems(Startup, |mut store: ResMut<GizmoConfigStore>| {
                let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
                config.line.width = 2.5;
                config.depth_bias = -0.0005;
            })
            .add_systems(Update, hover.before(super::tools::act))
            .add_systems(PostUpdate, draw.after(TransformSystems::Propagate));
    }
}

/// What the overlay shows, and how far out.
#[derive(Resource)]
pub struct Layers {
    pub shown: HashMap<Class, bool>,
    pub points: bool,
    /// Ways are drawn within this distance of the camera (m).
    pub radius: f32,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            shown: Class::ALL.iter().map(|&c| (c, c != Class::Other)).collect(),
            points: true,
            radius: 350.0,
        }
    }
}

impl Layers {
    pub fn shows(&self, class: Class) -> bool {
        self.shown.get(&class).copied().unwrap_or(true)
    }
}

/// Ways' draped lines, kept until the map file or the terrain changes.
#[derive(Resource, Default)]
struct Drapes {
    revision: u64,
    terrain: Option<bevy::ecs::change_detection::Tick>,
    lines: HashMap<i64, Vec<Vec3>>,
}

/// A map point on the ground, lifted a little, where the overlay draws it.
pub fn lifted(terrain: &Terrain, p: Vec2) -> Vec3 {
    Vec3::new(p.x, terrain.sample(p.x, p.y).0 + LIFT, p.y)
}

/// Draped points along a polyline.
fn drape(terrain: &Terrain, points: &[Vec2]) -> Vec<Vec3> {
    let mut out = Vec::new();
    for pair in points.windows(2) {
        let steps = (pair[0].distance(pair[1]) / DRAPE).ceil().max(1.0) as usize;
        for k in 0..steps {
            out.push(lifted(
                terrain,
                pair[0].lerp(pair[1], k as f32 / steps as f32),
            ));
        }
    }
    if let Some(&last) = points.last() {
        out.push(lifted(terrain, last));
    }
    out
}

/// Where a ray from the camera meets the ground (distance and point), up to 3 km out:
/// steps that grow with the distance (a pixel there is bigger), then halving.
fn ground_hit(terrain: &Terrain, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
    let above = |t: f32| {
        let p = origin + dir * t;
        p.y - terrain.sample(p.x, p.z).0
    };
    if above(0.0) < 0.0 {
        return Some((0.0, origin));
    }
    let mut t0 = 0.0;
    while t0 < 3000.0 {
        let t1 = t0 + 0.2 + t0 * 0.004;
        if above(t1) < 0.0 {
            let (mut lo, mut hi) = (t0, t1);
            for _ in 0..14 {
                let mid = 0.5 * (lo + hi);
                if above(mid) < 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let t = 0.5 * (lo + hi);
            return Some((t, origin + dir * t));
        }
        t0 = t1;
    }
    None
}

/// A node's position, with the one being dragged where it's being dragged to.
pub fn node_pos(doc: &Doc, tools: &Tools, id: i64) -> Option<Vec2> {
    match &tools.drag {
        Some(drag) if drag.node == id && drag.moved => Some(drag.to),
        _ => doc.pos(id),
    }
}

/// The ground under the cursor, and the node or way segment nearest it on screen.
#[allow(clippy::too_many_arguments)]
fn hover(
    doc: Res<Doc>,
    index: Res<Index>,
    layers: Res<Layers>,
    terrain: Res<Terrain>,
    tools: Res<Tools>,
    egui: Res<EguiWantsInput>,
    view: Res<MapView>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<FlyCamera>>,
    mut hover: ResMut<Hover>,
) {
    *hover = Hover::default();
    let (Ok(window), Ok((camera, eye, projection))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    if egui.is_pointer_over_area() || view.open {
        return;
    }
    let Ok(ray) = camera.viewport_to_world(eye, cursor) else {
        return;
    };
    let Some((distance, point)) = ground_hit(&terrain, ray.origin, *ray.direction) else {
        return;
    };
    hover.ground = Some(point);
    hover.screen = Some(cursor);
    // Metres per pixel at the hit point.
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    let per_pixel = distance * 2.0 * (fov / 2.0).tan() / window.height().max(1.0);
    let tolerance = (PICK_PIXELS * per_pixel).max(0.25);
    let g = point.xz();
    let dragged = tools.drag.as_ref().map(|d| d.node);

    let mut best_node: Option<(f32, i64)> = None;
    let mut best_segment: Option<(f32, Target)> = None;
    let mut consider_node = |id: i64, p: Vec2| {
        let d = p.distance(g);
        if Some(id) != dragged && d < tolerance * 1.3 && best_node.is_none_or(|(b, _)| d < b) {
            best_node = Some((d, id));
        }
    };
    for way_id in index.ways_near(g, tolerance + 1.0) {
        let Some(way) = doc.way(way_id) else {
            continue;
        };
        if !layers.shows(Class::of(&way.meta.tags)) {
            continue;
        }
        let points: Vec<(i64, Vec2)> = way
            .refs
            .iter()
            .filter_map(|&r| Some((r, node_pos(&doc, &tools, r)?)))
            .collect();
        for &(id, p) in &points {
            consider_node(id, p);
        }
        for (i, pair) in points.windows(2).enumerate() {
            let (a, b) = (pair[0].1, pair[1].1);
            let t = ((g - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
            let at = a.lerp(b, t);
            let d = at.distance(g);
            if d < tolerance && best_segment.as_ref().is_none_or(|(bd, _)| d < *bd) {
                let target = Target::Segment {
                    way: way_id,
                    index: i,
                    at,
                };
                best_segment = Some((d, target));
            }
        }
    }
    if layers.points {
        for id in index.points_near(g, tolerance + 1.0) {
            if let Some(p) = node_pos(&doc, &tools, id) {
                consider_node(id, p);
            }
        }
    }
    hover.target = best_node
        .map(|(_, id)| Target::Node(id))
        .or(best_segment.map(|(_, t)| t));
}

#[allow(clippy::too_many_arguments)]
fn draw(
    doc: Res<Doc>,
    index: Res<Index>,
    layers: Res<Layers>,
    terrain: Res<Terrain>,
    tools: Res<Tools>,
    selection: Res<Selection>,
    hover: Res<Hover>,
    mut drapes: ResMut<Drapes>,
    cameras: Query<&GlobalTransform, With<FlyCamera>>,
    mut gizmos: Gizmos,
) {
    let Ok(eye) = cameras.single() else {
        return;
    };
    let terrain_tick = terrain.last_changed();
    if drapes.revision != doc.revision || drapes.terrain != Some(terrain_tick) {
        drapes.lines.clear();
        drapes.revision = doc.revision;
        drapes.terrain = Some(terrain_tick);
    }
    let center = eye.translation().xz();
    let selected_way = selection.way;
    let hovered_way = match hover.target {
        Some(Target::Segment { way, .. }) => Some(way),
        _ => None,
    };

    for way_id in index.ways_near(center, layers.radius) {
        let Some(way) = doc.way(way_id) else {
            continue;
        };
        let class = Class::of(&way.meta.tags);
        if !layers.shows(class) && selected_way != Some(way_id) {
            continue;
        }
        let dragging = tools
            .drag
            .as_ref()
            .is_some_and(|d| d.moved && way.refs.contains(&d.node));
        let mut color = class.color();
        if class == Class::Road && !baked_road(&way.meta.tags) {
            // Paths and the like: not built.
            color = color.darker(0.35);
        }
        if Some(way_id) == selected_way {
            color = Color::WHITE;
        } else if Some(way_id) == hovered_way {
            color = Color::srgb(0.4, 1.0, 1.0);
        }
        if dragging {
            let points: Vec<Vec2> = way
                .refs
                .iter()
                .filter_map(|&r| node_pos(&doc, &tools, r))
                .collect();
            gizmos.linestrip(drape(&terrain, &points), color);
            continue;
        }
        let line = drapes
            .lines
            .entry(way_id)
            .or_insert_with(|| drape(&terrain, &doc.points(way)));
        gizmos.linestrip(line.iter().copied(), color);
    }

    // The selected way's nodes.
    if let Some(way) = selected_way.and_then(|w| doc.way(w)) {
        for &r in &way.refs {
            if let Some(p) = node_pos(&doc, &tools, r) {
                let shared = doc.ways_of(r).len() > 1;
                let color = if shared {
                    Color::srgb(1.0, 0.6, 0.2)
                } else {
                    Color::WHITE
                };
                gizmos.sphere(lifted(&terrain, p), 0.35, color);
            }
        }
    }

    if layers.points {
        for id in index.points_near(center, layers.radius) {
            let (Some(node), Some(p)) = (doc.node(id), node_pos(&doc, &tools, id)) else {
                continue;
            };
            let Some(kind) = PointKind::of(&node.meta.tags) else {
                continue;
            };
            let base = lifted(&terrain, p);
            let top = match node
                .meta
                .tags
                .get("ele")
                .and_then(|e| e.parse::<f32>().ok())
            {
                Some(ele) => Vec3::new(base.x, ele - terrain.base, base.z),
                None => base + Vec3::Y * POST,
            };
            gizmos.line(base, top, kind.color());
            gizmos.sphere(top, 0.3, kind.color());
        }
    }

    match hover.target {
        Some(Target::Node(id)) => {
            if let Some(p) = node_pos(&doc, &tools, id) {
                gizmos.sphere(lifted(&terrain, p), 0.6, Color::srgb(0.4, 1.0, 1.0));
            }
        }
        Some(Target::Segment { at, .. }) => {
            gizmos.sphere(lifted(&terrain, at), 0.25, Color::srgb(0.4, 1.0, 1.0));
        }
        None => {}
    }
    if let Some(node) = selection.node
        && let Some(p) = node_pos(&doc, &tools, node)
    {
        let at = lifted(&terrain, p);
        gizmos.sphere(at, 0.7, Color::srgb(1.0, 0.2, 0.8));
        gizmos.line(at, at + Vec3::Y * 4.0, Color::srgb(1.0, 0.2, 0.8));
    }

    // The way being drawn, and on to the cursor.
    if !tools.drawing.is_empty() {
        let mut points: Vec<Vec2> = tools.drawing.iter().filter_map(|p| p.at(&doc)).collect();
        if let Some(ground) = hover.ground {
            points.push(match hover.target {
                Some(Target::Node(id)) => doc.pos(id).unwrap_or(ground.xz()),
                Some(Target::Segment { at, .. }) => at,
                None => ground.xz(),
            });
        }
        let color = Color::srgb(1.0, 0.3, 0.9);
        gizmos.linestrip(drape(&terrain, &points), color);
        for p in tools.drawing.iter().filter_map(|p| p.at(&doc)) {
            gizmos.sphere(lifted(&terrain, p), 0.4, color);
        }
        if let Some(DrawPoint::New(first)) = tools.drawing.first()
            && tools.tool.closed()
        {
            gizmos.sphere(lifted(&terrain, *first), 0.8, color);
        }
    }
}
