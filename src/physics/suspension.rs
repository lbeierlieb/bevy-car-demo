//! Double A-arm (double wishbone) suspension as a 1-DOF kinematic linkage.
//!
//! Seen from the front, chassis, lower arm, upright and upper arm form a four-bar linkage:
//! the lower arm angle θ fixes everything else. Each corner integrates θ (and the wheel's
//! spin) at a high rate against a frozen chassis, and reports the force it exerts on the
//! chassis back to the caller.

use bevy::prelude::*;

use super::tire::{Patch, RIB_AT, RIBS, TireForce, TireParams, Tread};
use crate::terrain::Terrain;

const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// Lower arm travel allowed either side of the design angle before the bump stops engage.
const TRAVEL_LIMIT: f32 = 0.25;
/// Bump stop stiffness, in N·m per radian of arm rotation past the limit.
const BUMP_STOP: f32 = 60_000.0;

/// Linkage points in the front-view plane of a right-hand corner (x outward from the car's
/// centreline, y up from the centre of mass), at the design ride height.
/// Left corners are mirrored.
#[derive(Clone, Copy, Debug)]
pub struct ArmGeometry {
    pub lower_pivot: Vec2,
    pub lower_ball: Vec2,
    pub upper_pivot: Vec2,
    pub upper_ball: Vec2,
    pub wheel_center: Vec2,
    /// Coilover top mount on the chassis.
    pub spring_top: Vec2,
    /// Where the coilover attaches to the lower arm, as a fraction from pivot to ball joint.
    pub spring_arm_fraction: f32,
    /// Camber at design ride height, in radians (negative: top of the wheel leans inward).
    pub static_camber: f32,
    /// Toe of each wheel, in radians (positive: toe-in, the front of the wheel points
    /// inward).
    pub static_toe: f32,
}

impl Default for ArmGeometry {
    fn default() -> Self {
        Self {
            lower_pivot: Vec2::new(0.30, -0.22),
            lower_ball: Vec2::new(0.78, -0.22),
            // A shorter upper arm, angled so the arms converge inboard: negative camber gain
            // in bump and a roll centre a little above the ground.
            upper_pivot: Vec2::new(0.42, 0.03),
            upper_ball: Vec2::new(0.72, 0.08),
            wheel_center: Vec2::new(0.84, -0.09),
            spring_top: Vec2::new(0.55, 0.20),
            spring_arm_fraction: 0.7,
            static_camber: -0.02,
            static_toe: 0.0,
        }
    }
}

/// Solved linkage in the 2D front-view plane.
struct Linkage {
    lower_ball: Vec2,
    upper_ball: Vec2,
    wheel_center: Vec2,
    spring_bottom: Vec2,
    /// Upright rotation relative to the design pose (counter-clockwise from the front).
    upright_angle: f32,
}

impl ArmGeometry {
    /// Lower arm angle at design ride height.
    pub fn design_angle(&self) -> f32 {
        (self.lower_ball - self.lower_pivot).to_angle()
    }

    fn solve(&self, theta: f32) -> Linkage {
        let lower_len = self.lower_pivot.distance(self.lower_ball);
        let upper_len = self.upper_pivot.distance(self.upper_ball);
        let upright_len = self.lower_ball.distance(self.upper_ball);

        let lower_ball = self.lower_pivot + Vec2::from_angle(theta) * lower_len;
        let upper_ball = circle_intersection(lower_ball, upright_len, self.upper_pivot, upper_len);
        let upright_angle = (self.upper_ball - self.lower_ball).angle_to(upper_ball - lower_ball);
        Linkage {
            lower_ball,
            upper_ball,
            wheel_center: lower_ball
                + Vec2::from_angle(upright_angle).rotate(self.wheel_center - self.lower_ball),
            spring_bottom: self.lower_pivot.lerp(lower_ball, self.spring_arm_fraction),
            upright_angle,
        }
    }
}

/// Of the two intersections of circles (a, ra) and (b, rb), the one that lies outboard
/// of the line a→b (which is how the upper ball joint sits relative to the lower one).
fn circle_intersection(a: Vec2, ra: f32, b: Vec2, rb: f32) -> Vec2 {
    let d = a.distance(b);
    let dir = (b - a) / d;
    let along = (ra * ra - rb * rb + d * d) / (2.0 * d);
    let across = (ra * ra - along * along).max(0.0).sqrt();
    a + dir * along - dir.perp() * across
}

/// The linkage placed in the chassis frame for one corner.
#[derive(Clone, Copy, Debug, Default)]
pub struct CornerPose {
    pub lower_pivot: Vec3,
    pub upper_pivot: Vec3,
    pub lower_ball: Vec3,
    pub upper_ball: Vec3,
    pub wheel_center: Vec3,
    pub spring_top: Vec3,
    pub spring_bottom: Vec3,
    /// Orientation of the upright and wheel relative to the chassis (camber, then steer).
    pub upright: Quat,
    /// Camber relative to the chassis, in radians.
    pub camber: f32,
}

impl CornerPose {
    /// The point on the upright that sits at the bottom of the tire.
    fn contact_point(&self, radius: f32) -> Vec3 {
        self.wheel_center + self.upright * Vec3::new(0.0, -radius, 0.0)
    }

    pub fn spring_length(&self) -> f32 {
        self.spring_top.distance(self.spring_bottom)
    }
}

/// Suspension and wheel parameters shared by the corners of one axle.
#[derive(Clone, Copy, Debug)]
pub struct AxleSetup {
    pub geometry: ArmGeometry,
    /// Coilover spring rate (N/m) and damping (N·s/m), measured along the coilover.
    pub spring_rate: f32,
    pub damping: f32,
    pub unsprung_mass: f32,
    pub wheel_inertia: f32,
    pub tire: TireParams,
}

impl AxleSetup {
    fn pose(&self, theta: f32, steer: f32, side: f32, axle_z: f32) -> CornerPose {
        let g = &self.geometry;
        let l = g.solve(theta);
        let to_3d = |p: Vec2| Vec3::new(side * p.x, p.y, axle_z);
        let camber = g.static_camber - l.upright_angle;
        let (lower_ball, upper_ball) = (to_3d(l.lower_ball), to_3d(l.upper_ball));
        // Steering turns the upright about the kingpin axis through both ball joints, from
        // its toe angle (positive steer turns left, towards -x, the left side).
        let steer = steer + side * g.static_toe;
        let steer = Quat::from_axis_angle((upper_ball - lower_ball).normalize(), steer);
        CornerPose {
            lower_pivot: to_3d(g.lower_pivot),
            upper_pivot: to_3d(g.upper_pivot),
            lower_ball,
            upper_ball,
            wheel_center: lower_ball + steer * (to_3d(l.wheel_center) - lower_ball),
            spring_top: to_3d(g.spring_top),
            spring_bottom: to_3d(l.spring_bottom),
            upright: steer * Quat::from_rotation_z(-side * camber),
            camber,
        }
    }

    /// Sets the spring and damper for a ride `frequency` (Hz) and damping ratio of
    /// `corner_mass` of sprung mass on the wheel, through the linkage's motion ratio.
    pub fn tune(&mut self, frequency: f32, damping_ratio: f32, corner_mass: f32) {
        let d = Derivatives::at(self, self.geometry.design_angle(), 0.0, 1.0, 0.0);
        // Spring travel per wheel travel.
        let ratio = (d.length / d.contact.y).powi(2);
        let wheel_rate = corner_mass * (std::f32::consts::TAU * frequency).powi(2);
        self.spring_rate = wheel_rate / ratio;
        self.damping = 2.0 * damping_ratio * (wheel_rate * corner_mass).sqrt() / ratio;
    }

    /// Coilover free length that holds the design ride height under `corner_mass` of sprung
    /// mass: the spring's generalised force balances the tire load at the design angle.
    pub fn spring_rest_length(&self, corner_mass: f32) -> f32 {
        let theta = self.geometry.design_angle();
        let d = Derivatives::at(self, theta, 0.0, 1.0, 0.0);
        let load = (corner_mass + self.unsprung_mass) * -GRAVITY.y;
        let q_tire = load * d.contact.y + self.unsprung_mass * GRAVITY.y * d.center.y;
        let preload = -q_tire / d.length;
        d.pose.spring_length() + preload / self.spring_rate
    }
}

/// The pose plus derivatives of the key points with respect to θ (by central differences).
struct Derivatives {
    pose: CornerPose,
    center: Vec3,
    contact: Vec3,
    length: f32,
}

impl Derivatives {
    fn at(axle: &AxleSetup, theta: f32, steer: f32, side: f32, axle_z: f32) -> Self {
        const EPS: f32 = 1e-4;
        let r = axle.tire.radius;
        let pose = axle.pose(theta, steer, side, axle_z);
        let plus = axle.pose(theta + EPS, steer, side, axle_z);
        let minus = axle.pose(theta - EPS, steer, side, axle_z);
        Self {
            center: (plus.wheel_center - minus.wheel_center) / (2.0 * EPS),
            contact: (plus.contact_point(r) - minus.contact_point(r)) / (2.0 * EPS),
            length: (plus.spring_length() - minus.spring_length()) / (2.0 * EPS),
            pose,
        }
    }
}

/// The chassis state a corner integrates against during its substeps.
pub struct ChassisFrame {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

impl ChassisFrame {
    fn point_velocity(&self, point: Vec3) -> Vec3 {
        self.velocity + self.angular_velocity.cross(point - self.position)
    }
}

/// Where and how the tire touches the ground, for telemetry and debug drawing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    pub point: Vec3,
    pub normal: Vec3,
    pub load: f32,
    /// What each rib carries and grips.
    pub patch: Patch,
    pub tire: TireForce,
    /// Total tire force in world space.
    pub force: Vec3,
}

#[derive(Clone, Debug)]
pub struct Corner {
    /// +1 for right-hand corners, -1 for left.
    pub side: f32,
    /// Longitudinal position of the axle in the chassis frame.
    pub axle_z: f32,
    pub front: bool,
    /// Lower arm angle and its rate.
    pub theta: f32,
    pub theta_dot: f32,
    /// Wheel spin rate (rad/s, positive rolls forward) and accumulated angle for rendering.
    pub omega: f32,
    pub spin: f32,
    /// The tire's rubber: temperatures and wear.
    pub tread: Tread,
    /// The slips the tire acts on (the slip ratio, and the slip angle's tangent): they lag
    /// the patch's motion as the carcass flexes, see `TireParams::relax`.
    pub slips: [f32; 2],
    // Inputs, set by the car before each step.
    pub steer: f32,
    pub drive_torque: f32,
    /// Extra inertia coupled to the wheel by the drivetrain (the engine, through the gears).
    pub drive_inertia: f32,
    pub brake_torque: f32,
    // Outputs of the last substep.
    pub pose: CornerPose,
    pub contact: Option<Contact>,
    pub travel: f32,
}

impl Corner {
    pub fn new(axle: &AxleSetup, side: f32, axle_z: f32, front: bool) -> Self {
        let theta = axle.geometry.design_angle();
        Self {
            side,
            axle_z,
            front,
            theta,
            theta_dot: 0.0,
            omega: 0.0,
            spin: 0.0,
            tread: Tread::default(),
            slips: [0.0; 2],
            steer: 0.0,
            drive_torque: 0.0,
            drive_inertia: 0.0,
            brake_torque: 0.0,
            pose: axle.pose(theta, 0.0, side, axle_z),
            contact: None,
            travel: 0.0,
        }
    }

    /// Advances the corner by `dt` against a frozen chassis, adding the force it exerts on the
    /// chassis (and the torque of that force about `chassis.position`) to `force`/`torque`.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        axle: &AxleSetup,
        spring_rest_length: f32,
        chassis: &ChassisFrame,
        terrain: &Terrain,
        force: &mut Vec3,
        torque: &mut Vec3,
    ) {
        let tire = &axle.tire;
        let d = Derivatives::at(axle, self.theta, self.steer, self.side, self.axle_z);
        let pose = d.pose;
        let to_world = |p: Vec3| chassis.position + chassis.rotation * p;

        let center = to_world(pose.wheel_center);
        let axle_dir = chassis.rotation * (pose.upright * Vec3::X);
        let hub_velocity =
            chassis.point_velocity(center) + chassis.rotation * (d.center * self.theta_dot);

        // Each rib's lowest point (perpendicular to the axle, towards the road), and how far
        // it reaches into the ground below it along the road's normal.
        let up = chassis.rotation * Vec3::Y;
        let below = center - (up - axle_dir * axle_dir.dot(up)).normalize() * tire.radius;
        let (_, n) = terrain.sample(below.x, below.z);
        let down = -(n - axle_dir * axle_dir.dot(n)).normalize();
        let outside = axle_dir * self.side;
        let ribs = RIB_AT.map(|u| center + outside * (u * tire.width / 2.0) + down * tire.radius);
        let depth = ribs.map(|q| ((terrain.sample(q.x, q.z).0 - q.y) * n.y).min(tire.radius));

        // Without ground contact the wheel just spins up under drive torque.
        let inertia = axle.wheel_inertia + self.drive_inertia;
        let mut omega = self.omega + self.drive_torque / inertia * dt;
        let touching = depth.iter().any(|&d| d > 0.0);
        let patch = touching.then(|| {
            let outward = self.contact.map_or(0.0, |c| c.tire.fy) * self.side;
            let camber = -axle_dir.dot(n).clamp(-1.0, 1.0).asin();
            let patch = tire.patch(depth, -hub_velocity.dot(n), outward, camber, &self.tread);
            let forward = n.cross(axle_dir).normalize();
            let right = forward.cross(n);
            let (vx, vy) = (hub_velocity.dot(forward), hub_velocity.dot(right));
            // The carcass flexes and the patch takes up the slips as it rolls.
            let relaxation = tire.relaxation(&patch, &self.tread, self.slips);
            let slips = self.slips;
            let force = |omega| {
                let slips = tire.relax(slips, vx, vy, omega, &relaxation, dt);
                tire.relaxed_force(vx, vy, omega, slips, &relaxation, &patch)
            };

            // Wheel spin is the stiffest part of the system: update it implicitly by
            // linearising the longitudinal force (through the slip it takes up this step)
            // around the current spin rate.
            let f0 = force(self.omega);
            let nudged = force(self.omega + 0.01);
            let df_domega = (nudged.fx - f0.fx) / 0.01;
            omega = self.omega
                + dt * (self.drive_torque - tire.radius * f0.fx)
                    / (inertia + dt * tire.radius * df_domega.max(0.0));
            (patch, n, forward, right, vx, vy, relaxation)
        });
        self.omega = apply_brake(omega, self.brake_torque, inertia, dt);
        self.spin += self.omega * dt;

        let omega = self.omega;
        let speed = hub_velocity.length();
        let Some((patch, normal, forward, right, vx, vy, relaxation)) =
            patch.filter(|p| p.0.load() > 0.0)
        else {
            // Off the ground, the carcass springs back.
            self.slips = [0.0; 2];
            tire.heat(&mut self.tread, None, 0.0, 0.0, speed, dt);
            self.contact = None;
            return self.finish(
                dt,
                axle,
                spring_rest_length,
                chassis,
                &d,
                None,
                force,
                torque,
            );
        };
        self.slips = tire.relax(self.slips, vx, vy, omega, &relaxation, dt);
        let tire_force = tire.relaxed_force(vx, vy, omega, self.slips, &relaxation, &patch);
        // The rubber slides at the difference between the wheel's rim and the road along,
        // and at the sideways speed across.
        let slide_x = omega * tire.radius - vx;
        tire.heat(
            &mut self.tread,
            Some((&tire_force, &patch)),
            slide_x,
            vy,
            vx,
            dt,
        );
        let load = patch.load();
        // The patch's centre: the ribs' points on the road, by their load.
        let point = (0..RIBS)
            .map(|rib| (ribs[rib] + normal * depth[rib]) * patch.loads[rib])
            .sum::<Vec3>()
            / load;
        let contact = Contact {
            point,
            normal,
            load,
            patch,
            tire: tire_force,
            force: normal * load + forward * tire_force.fx + right * tire_force.fy,
        };
        self.contact = Some(contact);
        self.finish(
            dt,
            axle,
            spring_rest_length,
            chassis,
            &d,
            Some(contact),
            force,
            torque,
        );
    }

    /// The rest of a step once the tire's force is known: the linkage moves under it, the
    /// spring, the damper and gravity, and passes it on to the chassis.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self,
        dt: f32,
        axle: &AxleSetup,
        spring_rest_length: f32,
        chassis: &ChassisFrame,
        d: &Derivatives,
        contact: Option<Contact>,
        force: &mut Vec3,
        torque: &mut Vec3,
    ) {
        let pose = d.pose;
        let to_world = |p: Vec3| chassis.position + chassis.rotation * p;
        let center = to_world(pose.wheel_center);
        let tire_force = contact.map_or(Vec3::ZERO, |c| c.force);

        // Generalised force on θ by virtual work: each force times the θ-derivative of the
        // point it acts on. The spring/damper act along the coilover.
        let to_local = chassis.rotation.inverse();
        let spring = axle.spring_rate * (spring_rest_length - pose.spring_length());
        let limit = self.theta - axle.geometry.design_angle();
        let bump_stop = -BUMP_STOP * (limit - limit.clamp(-TRAVEL_LIMIT, TRAVEL_LIMIT));
        let q = (to_local * tire_force).dot(d.contact)
            + (to_local * GRAVITY * axle.unsprung_mass).dot(d.center)
            + spring * d.length
            - axle.damping * d.length * d.length * self.theta_dot
            + bump_stop;
        let inertia = axle.unsprung_mass * d.center.length_squared();
        let theta_ddot = q / inertia;
        self.theta_dot += theta_ddot * dt;
        self.theta += self.theta_dot * dt;

        // Everything acting on the unsprung mass that doesn't accelerate it goes into the
        // chassis: the tire force at the contact patch, plus gravity minus inertia at the hub.
        let hub_force = GRAVITY * axle.unsprung_mass
            - chassis.rotation * (d.center * theta_ddot) * axle.unsprung_mass;
        let contact_point = contact.map_or(center, |c| c.point);
        *force += tire_force + hub_force;
        *torque += (contact_point - chassis.position).cross(tire_force)
            + (center - chassis.position).cross(hub_force);

        self.pose = pose;
        self.travel = pose.wheel_center.y - axle.geometry.wheel_center.y;
    }
}

/// Brakes act as friction: they slow the wheel down but never reverse it.
fn apply_brake(omega: f32, brake_torque: f32, inertia: f32, dt: f32) -> f32 {
    let dw = brake_torque / inertia * dt;
    omega.signum() * (omega.abs() - dw).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axle() -> AxleSetup {
        AxleSetup {
            geometry: ArmGeometry::default(),
            spring_rate: 80_000.0,
            damping: 5_000.0,
            unsprung_mass: 40.0,
            wheel_inertia: 1.2,
            tire: TireParams::default(),
        }
    }

    #[test]
    fn solve_reproduces_design_pose() {
        let g = ArmGeometry::default();
        let l = g.solve(g.design_angle());
        assert!(l.upper_ball.distance(g.upper_ball) < 1e-5);
        assert!(l.wheel_center.distance(g.wheel_center) < 1e-5);
        assert!(l.upright_angle.abs() < 1e-5);
    }

    #[test]
    fn toe_in_points_both_wheels_inward() {
        let mut a = axle();
        a.geometry.static_toe = 1f32.to_radians();
        let theta = a.geometry.design_angle();
        for side in [-1.0, 1.0] {
            let forward = a.pose(theta, 0.0, side, 0.0).upright * Vec3::NEG_Z;
            // Forward is -z; the wheel on the +x side points a little towards -x.
            assert!(forward.x * side < -0.01, "side {side}: {forward}");
        }
    }

    #[test]
    fn short_upper_arm_gains_negative_camber_in_bump() {
        let a = axle();
        let theta = a.geometry.design_angle();
        let bump = a.pose(theta + 0.1, 0.0, 1.0, 0.0);
        assert!(bump.camber < a.geometry.static_camber);
        assert!(bump.wheel_center.y > a.geometry.wheel_center.y);
    }

    #[test]
    fn tuned_springs_hold_the_ride_frequency() {
        // The stock four-bar: 275 kg a corner at about 1.7 Hz takes a spring a little
        // softer than the 90 kN/m it used to have, and four times the mass needs four
        // times the spring.
        let mut a = axle();
        a.tune(1.7, 0.3, 275.0);
        assert!(
            (55_000.0..80_000.0).contains(&a.spring_rate),
            "{}",
            a.spring_rate
        );
        assert!((2_500.0..5_000.0).contains(&a.damping), "{}", a.damping);
        let spring = a.spring_rate;
        a.tune(1.7, 0.3, 1100.0);
        assert!((a.spring_rate / spring - 4.0).abs() < 1e-3);
    }

    #[test]
    fn preload_compresses_spring() {
        let a = axle();
        let pose = a.pose(a.geometry.design_angle(), 0.0, 1.0, 0.0);
        assert!(a.spring_rest_length(300.0) > pose.spring_length());
    }
}
