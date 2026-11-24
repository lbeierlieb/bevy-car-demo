//! What the race editor shows: each grid spot's box (as big as the biggest car; red where
//! two overlap) with an arrow the way it faces, the discs where things were clicked, the
//! gates as a race shows them, the route along the roads (red straight lines between the
//! checkpoints while it can't be found), numbers over everything, and all of it on the
//! map (M).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::Draft;
use super::edit::{EditSet, Pointer, Selected, Target, on_ground};
use crate::chassis::CHASSIS;
use crate::editor::camera::FlyCamera;
use crate::events::race::{self, GridSpot};
use crate::events::racing::{GateParts, spawn_gate};
use crate::map::{MapLine, MapLines, MapMark, MapMarks, MapView};
use crate::terrain::Terrain;

/// The discs: radius and thickness (m), how much bigger under the pointer.
const DISC: f32 = 0.9;
const DISC_THICK: f32 = 0.08;
const DISC_HOVER: f32 = 1.4;
/// The route is drawn this far over the road (m).
const LIFT: f32 = 0.3;
/// Numbers show within this distance (m).
const LABEL_RANGE: f32 = 800.0;
/// Colours: grid spots, checkpoints, the first checkpoint (the start), the route.
const SPOT: [u8; 3] = [120, 170, 255];
const CHECK: [u8; 3] = [255, 210, 40];
const FIRST: [u8; 3] = [80, 255, 80];
const ROUTE: [u8; 3] = [60, 200, 255];

pub struct ShowPlugin;

impl Plugin for ShowPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CarBox::largest())
            .init_resource::<Shown>()
            .add_systems(Startup, make_looks)
            .add_systems(Update, (markers, gates, lines, map).after(EditSet))
            .add_systems(EguiPrimaryContextPass, labels);
    }
}

/// The box a grid spot is drawn as: the biggest car's (each half-extent the largest of
/// any chassis' factory body box: x across, y up, z along).
#[derive(Resource, Clone, Copy, Debug)]
pub struct CarBox {
    pub half: Vec3,
}

impl CarBox {
    fn largest() -> Self {
        let half = (CHASSIS.iter())
            .map(|chassis| crate::car::body_box(chassis, &chassis.wheels).half)
            .fold(Vec3::ZERO, Vec3::max);
        Self { half }
    }

    /// Whether the map point `p` is inside a spot's box, seen from above.
    pub fn covers(&self, spot: &GridSpot, p: Vec2) -> bool {
        let forward = spot.forward();
        let side = Vec2::new(-forward.y, forward.x);
        let d = p - spot.at;
        d.dot(forward).abs() <= self.half.z && d.dot(side).abs() <= self.half.x
    }

    /// The corners of a spot's box, seen from above.
    fn corners(&self, spot: &GridSpot) -> [Vec2; 4] {
        let forward = spot.forward() * self.half.z;
        let side = Vec2::new(-forward.y, forward.x).normalize() * self.half.x;
        [
            spot.at + forward + side,
            spot.at + forward - side,
            spot.at - forward - side,
            spot.at - forward + side,
        ]
    }

    /// Whether two spots' boxes overlap (separating axes, seen from above).
    pub fn overlap(&self, a: &GridSpot, b: &GridSpot) -> bool {
        let (ca, cb) = (self.corners(a), self.corners(b));
        let axes = [
            a.forward(),
            a.forward().perp(),
            b.forward(),
            b.forward().perp(),
        ];
        axes.iter().all(|&axis| {
            let span = |corners: &[Vec2; 4]| {
                let d = corners.map(|c| c.dot(axis));
                (d.iter().copied().fold(f32::INFINITY, f32::min))
                    ..(d.iter().copied().fold(f32::NEG_INFINITY, f32::max))
            };
            let (sa, sb) = (span(&ca), span(&cb));
            sa.start < sb.end && sb.start < sa.end
        })
    }
}

/// Meshes and materials.
#[derive(Resource)]
struct Looks {
    disc: Handle<Mesh>,
    car: Handle<Mesh>,
    spot: Handle<StandardMaterial>,
    check: Handle<StandardMaterial>,
    first: Handle<StandardMaterial>,
    box_fine: Handle<StandardMaterial>,
    box_selected: Handle<StandardMaterial>,
    box_overlapping: Handle<StandardMaterial>,
    /// The banners: start, the others, the finish.
    banners: [Handle<StandardMaterial>; 3],
    gate: GateParts,
}

/// What's on screen, and what it was made from.
#[derive(Resource, Default)]
struct Shown {
    markers_of: Option<(u64, Option<Target>, Option<usize>)>,
    markers: Vec<Entity>,
    gates_of: Option<(Vec<Vec2>, Vec<u16>)>,
    gates: Vec<Entity>,
    /// The route's line over the road.
    draped: Vec<Vec3>,
    map_of: Option<(u64, u64)>,
}

fn make_looks(
    mut commands: Commands,
    car: Res<CarBox>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut glow = |[r, g, b]: [u8; 3], alpha: f32| {
        let color = Color::srgb_u8(r, g, b);
        materials.add(StandardMaterial {
            base_color: color.with_alpha(alpha),
            emissive: color.to_linear() * 0.6,
            alpha_mode: if alpha < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            unlit: true,
            ..default()
        })
    };
    let looks = Looks {
        spot: glow(SPOT, 1.0),
        check: glow(CHECK, 1.0),
        first: glow(FIRST, 1.0),
        box_fine: glow([200, 220, 255], 0.25),
        box_selected: glow([150, 190, 255], 0.45),
        box_overlapping: glow([255, 60, 60], 0.45),
        banners: [
            glow([80, 255, 80], 0.85),
            glow([255, 255, 255], 0.5),
            glow([255, 210, 40], 0.9),
        ],
        disc: meshes.add(Cylinder::new(DISC, DISC_THICK)),
        car: meshes.add(Cuboid::from_size(car.half * 2.0)),
        gate: GateParts::new(&mut meshes, &mut materials),
    };
    commands.insert_resource(looks);
}

/// The boxes and discs, made afresh when anything about them changes.
#[allow(clippy::too_many_arguments)]
fn markers(
    mut commands: Commands,
    draft: Res<Draft>,
    pointer: Res<Pointer>,
    selected: Res<Selected>,
    terrain: Res<Terrain>,
    car: Res<CarBox>,
    looks: Option<Res<Looks>>,
    mut shown: ResMut<Shown>,
) {
    let Some(looks) = looks else {
        return;
    };
    let key = (draft.revision, pointer.hover, selected.0);
    if shown.markers_of == Some(key) {
        return;
    }
    shown.markers_of = Some(key);
    for entity in shown.markers.drain(..) {
        commands.entity(entity).despawn();
    }
    let plan = &draft.plan;
    let disc = |commands: &mut Commands, at: Vec2, material: &Handle<StandardMaterial>, hover| {
        let scale = if hover { DISC_HOVER } else { 1.0 };
        commands
            .spawn((
                Mesh3d(looks.disc.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(on_ground(&terrain, at) + Vec3::Y * DISC_THICK)
                    .with_scale(Vec3::new(scale, 1.0, scale)),
                NotShadowCaster,
            ))
            .id()
    };
    let mut entities = Vec::new();
    for (i, spot) in plan.grid.iter().enumerate() {
        let overlapping =
            (plan.grid.iter().enumerate()).any(|(j, other)| j != i && car.overlap(spot, other));
        let material = if overlapping {
            &looks.box_overlapping
        } else if selected.0 == Some(i) || pointer.hover == Some(Target::Spot(i)) {
            &looks.box_selected
        } else {
            &looks.box_fine
        };
        let ground = on_ground(&terrain, spot.at);
        entities.push(
            commands
                .spawn((
                    Mesh3d(looks.car.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(ground + Vec3::Y * car.half.y)
                        .with_rotation(Quat::from_rotation_y(spot.yaw())),
                    NotShadowCaster,
                ))
                .id(),
        );
        let hover = pointer.on_disc && pointer.hover == Some(Target::Spot(i));
        entities.push(disc(&mut commands, spot.at, &looks.spot, hover));
    }
    for (i, &p) in plan.checkpoints.iter().enumerate() {
        let material = if i == 0 { &looks.first } else { &looks.check };
        let hover = pointer.on_disc && pointer.hover == Some(Target::Check(i));
        entities.push(disc(&mut commands, p, material, hover));
    }
    shown.markers = entities;
}

/// The gates as a race shows them, at the checkpoints along the route found, made afresh
/// when it changes.
fn gates(
    mut commands: Commands,
    draft: Res<Draft>,
    terrain: Res<Terrain>,
    looks: Option<Res<Looks>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shown: ResMut<Shown>,
) {
    let Some(looks) = looks else {
        return;
    };
    let route = &draft.route;
    let key = (route.line.clone(), route.marks.clone());
    if shown.gates_of.as_ref() == Some(&key) {
        return;
    }
    shown.gates_of = Some(key);
    for entity in shown.gates.drain(..) {
        commands.entity(entity).despawn();
    }
    shown.draped = (route.line.iter())
        .map(|&p| on_ground(&terrain, p) + Vec3::Y * LIFT)
        .collect();
    let mut all = race::checkpoint_gates(route);
    if route.looped {
        // The first again at the end.
        all.pop();
    }
    let last = all.len().saturating_sub(1);
    shown.gates = (all.iter().enumerate())
        .map(|(i, gate)| {
            let banner = match i {
                0 if route.looped => &looks.banners[2],
                0 => &looks.banners[0],
                _ if i == last && !route.looped => &looks.banners[2],
                _ => &looks.banners[1],
            };
            spawn_gate(
                &mut commands,
                &mut meshes,
                &terrain,
                &looks.gate,
                gate,
                banner,
            )
        })
        .collect();
}

/// The route over the road (or straight red lines while it can't be found) and an arrow
/// the way each grid spot faces.
fn lines(
    draft: Res<Draft>,
    shown: Res<Shown>,
    terrain: Res<Terrain>,
    car: Res<CarBox>,
    mut gizmos: Gizmos,
) {
    let color = |[r, g, b]: [u8; 3]| Color::srgb_u8(r, g, b);
    if shown.draped.len() >= 2 {
        gizmos.linestrip(shown.draped.iter().copied(), color(ROUTE));
    } else if draft.plan.checkpoints.len() >= 2 {
        let mut points: Vec<Vec3> = (draft.plan.checkpoints.iter())
            .map(|&p| on_ground(&terrain, p) + Vec3::Y * LIFT)
            .collect();
        if draft.plan.looped {
            points.push(points[0]);
        }
        gizmos.linestrip(points, Color::srgb(1.0, 0.2, 0.2));
    }
    for spot in &draft.plan.grid {
        let top = on_ground(&terrain, spot.at) + Vec3::Y * (car.half.y * 2.0 + 0.05);
        let forward = spot.forward().extend(0.0).xzy();
        gizmos.arrow(
            top - forward * car.half.z * 0.6,
            top + forward * (car.half.z + 1.0),
            Color::WHITE,
        );
    }
}

/// The route, the checkpoints and the grid on the map (M).
fn map(
    draft: Res<Draft>,
    mut shown: ResMut<Shown>,
    mut lines: ResMut<MapLines>,
    mut marks: ResMut<MapMarks>,
) {
    let key = (draft.revision, draft.found());
    if shown.map_of == Some(key) {
        return;
    }
    shown.map_of = Some(key);
    let line = (draft.route.line.len() >= 2).then(|| MapLine {
        points: draft.route.line.clone(),
        color: ROUTE,
        width: 4.0,
    });
    lines.0.insert("race", line.into_iter().collect());
    let order = draft.pole_first();
    let spots = order.iter().enumerate().map(|(n, &i)| MapMark {
        at: draft.plan.grid[i].at,
        color: SPOT,
        label: format!("P{}", n + 1),
        label_below: 1.0,
    });
    let checks = (draft.plan.checkpoints.iter().enumerate()).map(|(i, &at)| MapMark {
        at,
        color: if i == 0 { FIRST } else { CHECK },
        label: checkpoint_label(&draft, i),
        label_below: 6.0,
    });
    marks.0.insert("race", spots.chain(checks).collect());
}

/// What a checkpoint is called on screen.
fn checkpoint_label(draft: &Draft, i: usize) -> String {
    let n = draft.plan.checkpoints.len();
    let looped = draft.plan.looped && n >= 2;
    match i {
        0 if looped => "1 Start/finish".into(),
        0 => "1 Start".into(),
        _ if i == n - 1 && !looped && n >= 2 => format!("{} Finish", i + 1),
        _ => format!("{}", i + 1),
    }
}

/// Numbers over the grid spots (pole first) and checkpoints.
fn labels(
    mut contexts: EguiContexts,
    draft: Res<Draft>,
    view: Res<MapView>,
    terrain: Res<Terrain>,
    car: Res<CarBox>,
    cameras: Query<(&Camera, &GlobalTransform), With<FlyCamera>>,
) -> Result {
    if view.open {
        return Ok(());
    }
    let Ok((camera, eye)) = cameras.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    let label = |at: Vec2, height: f32, text: String, [r, g, b]: [u8; 3]| {
        let top = on_ground(&terrain, at) + Vec3::Y * height;
        if top.distance(eye.translation()) > LABEL_RANGE {
            return;
        }
        let Ok(screen) = camera.world_to_viewport(eye, top) else {
            return;
        };
        let galley = painter.layout_no_wrap(text, FontId::proportional(15.0), Color32::WHITE);
        let rect = Align2::CENTER_BOTTOM
            .anchor_size(crate::menu::to_points(ctx, screen), galley.size())
            .expand(4.0);
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(170));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.5, Color32::from_rgb(r, g, b)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + egui::vec2(4.0, 4.0), galley, Color32::WHITE);
    };
    for (n, i) in draft.pole_first().into_iter().enumerate() {
        let spot = draft.plan.grid[i];
        label(spot.at, car.half.y * 2.0 + 0.6, format!("P{}", n + 1), SPOT);
    }
    for (i, &p) in draft.plan.checkpoints.iter().enumerate() {
        let color = if i == 0 { FIRST } else { CHECK };
        label(p, 6.5, checkpoint_label(&draft, i), color);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_boxes_overlap_when_they_touch() {
        let car = CarBox::largest();
        assert!(
            car.half.z > 2.0 && car.half.x > 0.8 && car.half.z < 3.5,
            "{car:?}"
        );
        let spot = |x: f32, z: f32, heading: f32| GridSpot {
            at: Vec2::new(x, z),
            heading,
        };
        let a = spot(0.0, 0.0, 90.0);
        assert!(car.overlap(&a, &spot(car.half.z * 2.0 - 0.1, 0.0, 90.0)));
        assert!(!car.overlap(&a, &spot(car.half.z * 2.0 + 0.1, 0.0, 90.0)));
        assert!(!car.overlap(&a, &spot(0.0, car.half.x * 2.0 + 0.1, 90.0)));
        assert!(
            car.overlap(&a, &spot(0.0, car.half.x * 2.0 + 0.1, 0.0)),
            "turned"
        );
        assert!(car.covers(&a, Vec2::new(car.half.z - 0.1, 0.0)));
        assert!(!car.covers(&a, Vec2::new(0.0, car.half.x + 0.1)));
    }
}
