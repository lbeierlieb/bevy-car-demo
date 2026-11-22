//! The rescue helicopter. Where the rules put the car on the nearest road (R), a
//! helicopter is called instead of the car jumping there: it flies in from ahead of the
//! car, holds over it and lets its cable down, lifts the car (turning it upright), carries
//! it to the road (not fast with a car on the cable: `CARRY_SPEED`), sets it down in the
//! lane, unhooks and flies off. Near a road it all takes about ten seconds, longer the
//! farther the road is. From the call until it's set down the driver's input does
//! nothing (`car::Rescued`, which also makes the car a ghost online) and R does nothing.
//!
//! The flight runs in fixed steps after the simulation, which keeps simulating the car
//! (its wheels hang and its engine runs) while the rescue puts its body where it hangs.
//! Its pose goes with our car's state to the other players (`Rescue::pose`), whose games
//! draw it.

use std::f32::consts::FRAC_PI_4;

use bevy::prelude::*;

use super::{HOOK, Heli, HeliPose, attitude, push, turn, yaw_of};
use crate::car::{ActiveCar, Away, Car, Rescued, heading};
use crate::physics::collision::Obb;
use crate::terrain::Terrain;

/// The phases' lengths (s): flying in, letting the cable down, lifting the car off (before
/// it moves away) and setting it down, each with this long more per metre it's lifted,
/// unhooking, flying off.
const APPROACH: f32 = 3.0;
const LOWER: f32 = 1.0;
const LIFT: f32 = 1.0;
const SET_DOWN: f32 = 1.1;
const PER_METRE: f32 = 0.12;
const UNHOOK: f32 = 0.8;
const DEPART: f32 = 8.0;
/// How far ahead of the car it comes from and how high above (m), and how fast it flies at
/// the start (m/s).
const COMES_FROM: f32 = 140.0;
const COMES_HIGH: f32 = 40.0;
const COMES_AT: f32 = 50.0;
/// How far it holds above the car's roof, the length of the cable it carries on (m), at
/// least; more where trees or houses stand around the car (it holds this far over them,
/// what's within `HOLD_REACH`, m), letting the cable down at `LOWER_RATE` (m/s).
const CABLE: f32 = 12.0;
const HOLD_CLEARANCE: f32 = 4.0;
const HOLD_REACH: f32 = 8.0;
const LOWER_RATE: f32 = 12.0;
/// How fast it carries a car on average over a long way (m/s), with this long more for
/// speeding up and slowing down (s).
const CARRY_SPEED: f32 = 10.0;
const CARRY_EXTRA: f32 = 1.0;
/// How high it lifts the car over the straight line to the road: a share of the way,
/// within these (m), and at least this far over the land, trees and houses in between,
/// what's within `LIFT_REACH` of the way (m).
const LIFT_SHARE: f32 = 0.4;
const LIFT_HEIGHT: (f32, f32) = (8.0, 25.0);
const LIFT_CLEARANCE: f32 = 4.0;
const LIFT_REACH: f32 = 4.0;
/// Flying off: how hard it speeds up (m/s²), how fast it climbs (m/s), how fast the cable
/// comes in (m/s).
const DEPART_ACCELERATION: f32 = 7.0;
const DEPART_CLIMB: f32 = 4.0;
const REEL: f32 = 8.0;
/// How fast it turns (rad/s).
const TURN: f32 = 1.2;
/// A car moved this far from where the rescue left it (by anything else: a round
/// starting, a lent car) is let go of (m).
const LET_GO: f32 = 2.0;

pub struct RescuePlugin;

impl Plugin for RescuePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rescue>()
            .add_systems(FixedPostUpdate, fly)
            .add_systems(Update, draw.after(crate::car::sync_transforms));
    }
}

/// The rescue under way, if any.
#[derive(Resource, Default)]
pub struct Rescue {
    flight: Option<Flight>,
}

impl Rescue {
    /// Calls the helicopter to carry `car` (its entity) to the road at `to` (xz), facing
    /// along `yaw`. Nothing if one is already under way.
    pub fn call(&mut self, entity: Entity, car: &Car, terrain: &Terrain, to: Vec2, yaw: f32) {
        if self.busy() {
            return;
        }
        let height = car.setup.build.chassis().cg_height + 0.15;
        let end = Vec3::new(
            to.x,
            terrain.ground(to.x, to.y, f32::INFINITY).height + height,
            to.y,
        );
        // From ahead of the car (where the chase camera looks), a little to the side.
        let forward = (car.body.rotation * Vec3::NEG_Z)
            .xz()
            .normalize_or(Vec2::NEG_Y);
        let from = Vec2::from_angle(FRAC_PI_4 * 0.7).rotate(forward) * COMES_FROM;
        let load = load_point(&car.setup.hull, car.body.position, car.body.rotation);
        // Clear of the trees and houses around the car (its skids are 1.3 m under its hook
        // and more), on a longer cable.
        let tops = terrain.obstacles.near(load.xz(), HOLD_REACH).map(|o| o.top);
        let cable = CABLE.max(tops.fold(f32::NEG_INFINITY, f32::max) + HOLD_CLEARANCE - load.y);
        let start = load + Vec3::new(from.x, COMES_HIGH + cable, from.y);
        let heli = Pose {
            position: start,
            velocity: Vec3::ZERO,
            yaw: yaw_of(-from),
            push: Vec3::ZERO,
            cable: 0.0,
        };
        self.flight = Some(Flight {
            car: entity,
            t: 0.0,
            to,
            yaw,
            end,
            start,
            cable,
            carry: None,
            heli,
            load: None,
            left_at: None,
            leaving: None,
        });
    }

    /// Whether the helicopter is on its way, carrying the car or flying off.
    pub fn busy(&self) -> bool {
        self.flight.is_some()
    }

    /// The helicopter's pose for the other players, while there is one, as of the latest
    /// simulation step's end, `time` (on the clock our car's states go on).
    pub fn pose(&self, time: f64) -> Option<HeliPose> {
        let flight = self.flight.as_ref()?;
        let heli = &flight.heli;
        Some(HeliPose {
            time,
            position: heli.position,
            velocity: heli.velocity,
            rotation: heli.rotation(),
            cable: heli.cable,
            hooked: flight.load.is_some(),
        })
    }
}

/// A rescue's flight.
struct Flight {
    /// The car's entity.
    car: Entity,
    /// Since it was called (s).
    t: f32,
    /// Where the car goes (xz) and faces, and where its centre of mass is then.
    to: Vec2,
    yaw: f32,
    end: Vec3,
    /// Where the helicopter came from.
    start: Vec3,
    /// How far over the car's roof it holds, the cable's length (m).
    cable: f32,
    /// The way the car goes, worked out once it's hooked on.
    carry: Option<Carry>,
    heli: Pose,
    /// Where the cable is hooked onto the car, while it is.
    load: Option<Vec3>,
    /// Where the rescue put the car's centre of mass last step.
    left_at: Option<Vec3>,
    /// When it flies off (s since the call): set down and unhooked, or let go of.
    leaving: Option<f32>,
}

/// The helicopter as it flies.
#[derive(Clone, Copy, Debug)]
struct Pose {
    position: Vec3,
    velocity: Vec3,
    yaw: f32,
    /// What it pushes with (smoothed, for its tilt).
    push: Vec3,
    cable: f32,
}

impl Pose {
    fn rotation(&self) -> Quat {
        attitude(self.yaw, self.push)
    }

    /// Moves to `position` over `dt`, turning towards `wanted` yaw if given.
    fn fly_to(&mut self, position: Vec3, wanted: Option<f32>, dt: f32) {
        let velocity = (position - self.position) / dt;
        let acceleration = (velocity - self.velocity) / dt;
        self.position = position;
        self.velocity = velocity;
        self.push = (self.push).lerp(push(velocity, acceleration), 1.0 - (-dt / 0.3).exp());
        if let Some(wanted) = wanted {
            self.yaw = turn(self.yaw, wanted, TURN * dt);
        }
    }
}

/// The car's way from where it was hooked on to the road.
struct Carry {
    /// Its centre of mass and orientation when hooked on, and the yaw upright then.
    from: Vec3,
    rotation: Quat,
    yaw: f32,
    /// How high it's lifted over the straight line at most (m).
    lift: f32,
    /// How long from lifting it to setting it down (s), of which lifting it off and
    /// setting it down.
    duration: f32,
    lifting: f32,
    setting_down: f32,
    /// When it was hooked on (s since the call).
    hooked: f32,
    /// The hook's offset from above the load (m), smoothed, from the swing.
    swing: Vec3,
}

impl Carry {
    fn new(car: &Car, flight: &Flight, terrain: &Terrain) -> Self {
        let from = car.body.position;
        let distance = from.xz().distance(flight.end.xz());
        let lift = (LIFT_SHARE * distance).clamp(LIFT_HEIGHT.0, LIFT_HEIGHT.1);
        // Clear of the land, trees and houses in between: the highest along the way, under
        // the bottom of the car hanging (about its centre of mass: the car's height).
        let steps = (distance / 2.0).ceil().max(1.0) as usize;
        let highest = (0..=steps)
            .map(|k| {
                let p = from.xz().lerp(flight.end.xz(), k as f32 / steps as f32);
                let tops = terrain.obstacles.near(p, LIFT_REACH).map(|o| o.top);
                tops.fold(terrain.sample(p.x, p.y).0, f32::max)
            })
            .fold(f32::NEG_INFINITY, f32::max);
        let line = from.y.max(flight.end.y);
        let lift = lift.max(highest + LIFT_CLEARANCE + 1.0 - line);
        let (lifting, setting_down) = (LIFT + PER_METRE * lift, SET_DOWN + PER_METRE * lift);
        Self {
            from,
            rotation: car.body.rotation,
            yaw: heading(car.body.rotation),
            lift,
            duration: lifting + CARRY_EXTRA + distance / CARRY_SPEED + setting_down,
            lifting,
            setting_down,
            hooked: flight.t,
            swing: Vec3::ZERO,
        }
    }

    /// Where the car is and how it's turned `s` seconds after it was hooked on.
    fn at(&self, s: f32, end: Vec3, yaw: f32) -> (Vec3, Quat) {
        let u = (s / self.duration).clamp(0.0, 1.0);
        let (a, b) = (
            self.lifting / self.duration,
            self.setting_down / self.duration,
        );
        // Along the way between lifting and setting down; up mostly before, down after.
        let along = smootherstep((u - a) / (1.0 - a - b));
        let middle = 1.0 - a - b;
        let up =
            smootherstep(u / (a + 0.15 * middle)) * smootherstep((1.0 - u) / (b + 0.15 * middle));
        let position = self.from.lerp(end, along) + Vec3::Y * self.lift * up;
        // Upright while it's lifted, then turned to face along the road.
        let upright = Quat::from_rotation_y(self.yaw);
        let rotation = if u < a {
            self.rotation.slerp(upright, smootherstep(u / a))
        } else {
            upright.slerp(Quat::from_rotation_y(yaw), along)
        };
        (position, rotation.normalize())
    }
}

fn smootherstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (6.0 * x - 15.0) + 10.0)
}

/// Where the cable hooks onto a car (its box `hull` placed at `position` turned by
/// `rotation`): above its centre, at the top of the box however it lies.
fn load_point(hull: &Obb, position: Vec3, rotation: Quat) -> Vec3 {
    let center = position + rotation * hull.center;
    let top: f32 = (0..3)
        .map(|i| (rotation * hull.axes[i]).y.abs() * hull.half[i])
        .sum();
    Vec3::new(position.x, center.y + top, position.z)
}

/// Flies the rescue on by a step, carrying the car while it's hooked on.
fn fly(
    time: Res<Time>,
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    away: Res<Away>,
    mut rescue: ResMut<Rescue>,
    mut rescued: ResMut<Rescued>,
    mut cars: Query<&mut Car>,
) {
    let dt = time.delta_secs();
    rescued.0 = false;
    let Some(flight) = &mut rescue.flight else {
        return;
    };
    flight.t += dt;
    let car = (cars.get_mut(flight.car).ok()).filter(|car| car.index == active.0 && !away.0);
    // Something else moved the car, or it's gone: the helicopter lets go and flies off.
    let moved = (car.as_ref().zip(flight.left_at))
        .is_some_and(|(car, at)| car.body.position.distance(at) > LET_GO);
    if (car.is_none() || moved) && flight.leaving.is_none() {
        flight.leaving = Some(flight.t);
        flight.load = None;
    }
    match (car, flight.leaving) {
        (Some(mut car), None) => {
            rescued.0 = true;
            carry(flight, &mut car, &terrain, dt);
        }
        _ => {
            if fly_off(flight, dt) {
                rescue.flight = None;
            }
        }
    }
}

/// Unhooked: the cable comes in, and it climbs away. Whether it's gone.
fn fly_off(flight: &mut Flight, dt: f32) -> bool {
    let leaving = flight.t - flight.leaving.unwrap_or(flight.t);
    let heli = &mut flight.heli;
    heli.cable = (heli.cable - REEL * dt).max(0.0);
    if leaving > DEPART {
        return true;
    }
    let forward = Quat::from_rotation_y(heli.yaw) * Vec3::NEG_Z;
    let speed =
        heli.velocity.dot(forward).max(0.0) + DEPART_ACCELERATION * dt * smootherstep(leaving);
    let climb = DEPART_CLIMB * smootherstep(leaving / 1.5);
    let position = heli.position + (forward * speed + Vec3::Y * climb) * dt;
    heli.fly_to(position, None, dt);
    false
}

/// Flies in and lets the cable down, then carries the car, then sets it down and lets go.
fn carry(flight: &mut Flight, car: &mut Car, terrain: &Terrain, dt: f32) {
    let t = flight.t;
    // From the call on: out of gear (an H-shifter's lever counts again once set down) and
    // the parking brake on.
    car.drivetrain.neutral();
    car.parking_brake = true;
    let load = load_point(&car.setup.hull, car.body.position, car.body.rotation);
    let lower = LOWER.max(flight.cable / LOWER_RATE);
    if t < APPROACH + lower {
        // Flying in to hold over the car (it may still be settling), then letting the
        // cable down to it.
        let hover = load + Vec3::Y * flight.cable - HOOK;
        let u = (t / APPROACH).min(1.0);
        // A cubic from where it came from, coming in fast, to a standstill over the car.
        let tangent = (hover - flight.start).normalize_or_zero() * COMES_AT * APPROACH;
        let h1 = 1.0 - 3.0 * u * u + 2.0 * u * u * u;
        let h2 = u * u * u - 2.0 * u * u + u;
        let position = flight.start * h1 + hover * (1.0 - h1) + tangent * h2;
        let wanted = (u < 0.8).then(|| yaw_of((hover - flight.heli.position).xz()));
        flight.heli.fly_to(position, wanted, dt);
        let lowered = ((t - APPROACH) / lower).clamp(0.0, 1.0);
        flight.heli.cable = (hover + HOOK).distance(load) * smootherstep(lowered);
        return;
    }
    // Hooked on: the rescue carries the car.
    if flight.carry.is_none() {
        flight.carry = Some(Carry::new(car, flight, terrain));
    }
    let carry = flight.carry.as_mut().unwrap();
    let s = t - carry.hooked;
    if s >= carry.duration {
        // Down: in the lane, let go of; the helicopter unhooks and flies off.
        car.place(terrain, flight.to, flight.yaw);
        car.drivetrain.neutral();
        flight.load = None;
        flight.leaving = Some(t + UNHOOK);
        return;
    }
    let (position, rotation) = carry.at(s, flight.end, flight.yaw);
    let (before, was) = (car.body.position, car.body.rotation);
    let body = &mut car.body;
    body.position = position;
    body.rotation = rotation;
    body.velocity = (position - before) / dt;
    body.angular_velocity = (rotation * was.inverse()).to_scaled_axis() / dt;
    flight.left_at = Some(position);
    let load = load_point(&car.setup.hull, position, rotation);
    flight.load = Some(load);
    // The helicopter over the load, ahead of it by how hard it pulls it along (as a load
    // swinging on a cable lags), on a cable as long as it was let down.
    let (ahead, _) = carry.at(s + 0.1, flight.end, flight.yaw);
    let (behind, _) = carry.at((s - 0.1).max(0.0), flight.end, flight.yaw);
    let acceleration = (ahead - 2.0 * position + behind) / 0.01;
    let lean = Vec3::new(acceleration.x, 0.0, acceleration.z) * (flight.cable / 9.81);
    let smoothing = 1.0 - (-dt / 0.4).exp();
    carry.swing = (carry.swing).lerp(lean.clamp_length_max(0.4 * flight.cable), smoothing);
    let cable = flight.heli.cable;
    let hanging = (cable * cable - carry.swing.length_squared())
        .max(1.0)
        .sqrt();
    let hook = load + carry.swing + Vec3::Y * hanging;
    let moving = (ahead - behind).xz() / 0.2;
    let wanted = (moving.length() > 2.0).then(|| yaw_of(moving));
    flight.heli.fly_to(hook - HOOK, wanted, dt);
}

/// The rescue helicopter where the flight has it.
#[allow(clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    rescue: Res<Rescue>,
    mut helis: Query<(Entity, &mut Heli, &mut Transform), With<OurRescue>>,
) {
    let existing = helis.iter_mut().next();
    let Some(flight) = &rescue.flight else {
        if let Some((entity, ..)) = existing {
            commands.entity(entity).despawn();
        }
        return;
    };
    let heli = Heli {
        cable: flight.heli.cable,
        load: flight.load,
    };
    let transform =
        Transform::from_translation(flight.heli.position).with_rotation(flight.heli.rotation());
    match existing {
        Some((_, mut drawn, mut at)) => {
            *drawn = heli;
            *at = transform;
        }
        None => {
            commands.spawn((heli, transform, OurRescue));
        }
    }
}

/// Our own rescue helicopter.
#[derive(Component)]
struct OurRescue;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;
    use crate::terrain::Spawn;

    /// Runs a rescue of a car `off` metres to the side of the road at the spawn, lying on
    /// its roof, back to the road there: how long until it's let go of, the car then, and
    /// where it was to go.
    fn rescue(off: f32) -> (f32, Car, Vec2, f32) {
        let terrain = Terrain::baked();
        let (spawn, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let side = Vec2::new(yaw.cos(), -yaw.sin());
        let p = spawn + off * side;
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, p.x, p.y, yaw);
        car.body.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(3.0);
        let (to, facing) = terrain
            .drive_spot(spawn, Vec2::new(-yaw.sin(), -yaw.cos()))
            .unwrap();
        let mut app = App::new();
        app.insert_resource(terrain)
            .init_resource::<ActiveCar>()
            .init_resource::<Away>()
            .init_resource::<Rescued>()
            .init_resource::<Rescue>()
            .insert_resource(Time::<()>::default())
            .add_systems(Update, (crate::sim::simulate, fly).chain());
        let entity = app.world_mut().spawn(car).id();
        app.world_mut()
            .resource_scope(|world, terrain: Mut<Terrain>| {
                let car = world.get::<Car>(entity).unwrap().clone();
                (world.resource_mut::<Rescue>()).call(entity, &car, &terrain, to, facing);
            });
        let dt = 1.0 / 120.0;
        let mut t = 0.0;
        let mut highest = f32::NEG_INFINITY;
        while app.world().resource::<Rescued>().0 || t < 0.1 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(dt));
            app.update();
            t += dt;
            highest = highest.max(app.world().get::<Car>(entity).unwrap().body.position.y);
            assert!(t < 120.0, "never let go");
        }
        let car = app.world().get::<Car>(entity).unwrap().clone();
        assert!(
            highest > car.body.position.y + LIFT_HEIGHT.0 - 1.0,
            "not lifted"
        );
        (t, car, to, facing)
    }

    #[test]
    fn the_helicopter_takes_the_car_to_the_road() {
        let (near, car, to, facing) = rescue(15.0);
        assert!(car.body.position.xz().distance(to) < 0.1);
        assert!((car.body.rotation * Vec3::Y).y > 0.99, "not upright");
        assert!((heading(car.body.rotation) - facing).abs() < 0.01);
        assert!((8.0..12.0).contains(&near), "{near} s 15 m from the road");
        let (far, car, to, _) = rescue(100.0);
        assert!(car.body.position.xz().distance(to) < 0.1);
        assert!((20.0..25.0).contains(&far), "{far} s 100 m from the road");
    }
}
