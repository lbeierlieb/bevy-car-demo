//! Debug gizmos: suspension linkage, raycasts, contact patches, tire forces, roll centres,
//! and the collision boxes of the cars and the obstacles near them.

use bevy::color::palettes::css;
use bevy::prelude::*;

use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Car};
use crate::physics::suspension::Corner;
use crate::terrain::Terrain;

/// Metres of arrow per newton of tire force.
const FORCE_SCALE: f32 = 1.0 / 4_000.0;
/// Obstacles within this distance of a car get their boxes drawn (m).
const OBSTACLE_RANGE: f32 = 30.0;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugView>()
            .add_systems(Startup, draw_on_top)
            .add_systems(
                Update,
                (toggle, draw.run_if(|view: Res<DebugView>| view.gizmos)),
            );
    }
}

/// What's shown: the gizmos (1) and the panel (2), both hidden at first.
#[derive(Resource, Default)]
pub struct DebugView {
    pub gizmos: bool,
    pub ui: bool,
}

fn draw_on_top(mut config: ResMut<GizmoConfigStore>) {
    config.config_mut::<DefaultGizmoConfigGroup>().0.depth_bias = -1.0;
}

/// 1 toggles the gizmos, 2 the panel.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<DebugView>) {
    if keys.just_pressed(KeyCode::Digit1) {
        view.gizmos = !view.gizmos;
    }
    if keys.just_pressed(KeyCode::Digit2) {
        view.ui = !view.ui;
    }
}

fn draw(
    cars: Query<&Car>,
    active: Res<ActiveCar>,
    cameras: Query<&ChaseCamera>,
    terrain: Res<Terrain>,
    mut gizmos: Gizmos,
) {
    // Gizmos draw on top, so they'd cover the dashboard of the car the camera sits in.
    let seated = cameras.iter().any(|camera| camera.driver());
    for car in &cars {
        for b in terrain
            .obstacles
            .near(car.body.position.xz(), OBSTACLE_RANGE)
        {
            for [from, to] in b.obb().edges() {
                gizmos.line(from, to, css::TOMATO);
            }
        }
    }
    for car in cars.iter().filter(|car| !(seated && car.index == active.0)) {
        let to_world = |p: Vec3| car.body.position + car.body.rotation * p;
        let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
        for [from, to] in hull.edges() {
            gizmos.line(from, to, css::ORANGE);
        }
        for corner in &car.corners {
            let p = &corner.pose;
            let radius = car.setup.axle(corner.front).tire.radius;
            for (pivot, ball) in [(p.lower_pivot, p.lower_ball), (p.upper_pivot, p.upper_ball)] {
                for dz in [-0.18, 0.18] {
                    gizmos.line(to_world(pivot + Vec3::Z * dz), to_world(ball), css::YELLOW);
                }
            }
            gizmos.line(to_world(p.lower_ball), to_world(p.upper_ball), css::WHITE);
            gizmos.line(
                to_world(p.spring_bottom),
                to_world(p.spring_top),
                css::ORANGE_RED,
            );

            let center = to_world(p.wheel_center);
            match corner.contact {
                Some(contact) => {
                    gizmos.line(center, contact.point, css::LIME);
                    gizmos.sphere(contact.point, 0.04, css::LIME);
                    let normal = contact.normal * contact.normal.dot(contact.force);
                    let grip = contact.force - normal;
                    gizmos.arrow(
                        contact.point,
                        contact.point + normal * FORCE_SCALE,
                        css::DEEP_SKY_BLUE,
                    );
                    gizmos.arrow(
                        contact.point,
                        contact.point + grip * FORCE_SCALE,
                        css::MAGENTA,
                    );
                }
                None => {
                    let down = car.body.rotation * (p.upright * Vec3::NEG_Y);
                    gizmos.line(center, center + down * radius, css::GRAY);
                }
            }
        }
        for axle in [&car.corners[0..2], &car.corners[2..4]] {
            if let Some(rc) = roll_centre(
                &axle[0],
                &axle[1],
                car.setup.axle(axle[0].front).tire.radius,
            ) {
                gizmos.sphere(to_world(rc), 0.06, css::AQUA);
            }
        }
    }
}

/// Front-view roll centre of an axle (in the chassis frame): each side's line from its
/// contact patch through the instant centre of its A-arms; the roll centre is where the
/// two lines cross.
fn roll_centre(left: &Corner, right: &Corner, radius: f32) -> Option<Vec3> {
    let line = |c: &Corner| {
        let p = &c.pose;
        let xy = |v: Vec3| v.xy();
        let ic = intersect(
            xy(p.lower_pivot),
            xy(p.lower_ball - p.lower_pivot),
            xy(p.upper_pivot),
            xy(p.upper_ball - p.upper_pivot),
        )?;
        let contact = xy(p.wheel_center) - Vec2::Y * radius;
        Some((contact, ic - contact))
    };
    let ((a, da), (b, db)) = (line(left)?, line(right)?);
    intersect(a, da, b, db).map(|rc| rc.extend(left.axle_z))
}

/// Intersection of the lines p + t·d and q + s·e, if they aren't parallel.
fn intersect(p: Vec2, d: Vec2, q: Vec2, e: Vec2) -> Option<Vec2> {
    let denom = d.perp_dot(e);
    (denom.abs() > 1e-6).then(|| p + d * (q - p).perp_dot(e) / denom)
}
