//! A player's side: say hello, send the car being driven (its looks when they change, its
//! state `SEND_RATE` times a second), and show the cars of the players nearby as proxies:
//! entities with their `Looks` and `Motion` (so they're drawn and heard like any car),
//! where they are by now: carried on from their latest state (dead reckoning), turning as
//! they turned and speeding up or slowing down as they did between their last two states
//! (`predict`). What's drawn closes on that smoothly (`Drawn`), so a new state that
//! disagrees doesn't make a car jump. A car only pushes itself off a proxy; the other
//! player's game does the same on their side. On a server the traffic is the server's:
//! its cars near us are shown the same way (`ServerTraffic`), and our own traffic stops.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::*;

use super::SEND_RATE;
use super::link::Link;
use super::protocol::{CarState, PROTOCOL, PlayerId, ToClient, ToServer, TrafficState};
use super::wire::{decode, encode};
use crate::car::{ActiveCar, Car};
use crate::looks::{Looks, Motion};
use crate::physics::collision::Obb;
use crate::physics::collision::{self, OBSTACLE};
use crate::physics::rigid_body::RigidBody;
use crate::terrain::Terrain;
use crate::traffic::{self, TrafficCar};

/// How long a car is carried on past its latest state; then it's held there (s). More
/// than the time between states far away (3 a second), and a lost one.
const PREDICT: f64 = 0.8;
/// How quickly what's drawn of a car closes on where it's predicted to be (s, the time
/// constant).
const SMOOTHING: f32 = 0.1;
/// Further than this from where it's predicted to be, a car is drawn there at once (m).
const SNAP: f32 = 5.0;
/// The step a car is carried on in (s).
const STEP: f32 = 1.0 / 120.0;
/// The most acceleration believed from two states (m/s²): more is a crash, which doesn't
/// go on.
const MOST_ACCELERATION: f32 = 15.0;
/// A proxy not heard of for this long is gone (out of range) (s).
const GONE: f64 = 2.0;
/// States kept per car: the last two give the acceleration.
const KEPT: usize = 2;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (receive, dress_proxies, send, (move_proxies, remove_proxies))
                .chain()
                .after(crate::looks::show_cars)
                .run_if(resource_exists::<Connection>),
        )
        .init_resource::<TrafficClock>()
        .init_resource::<traffic::Source>()
        .add_systems(PreUpdate, (traffic_source, move_traffic).chain())
        .add_systems(
            FixedUpdate,
            push_off_proxies
                .after(crate::sim::simulate)
                .run_if(resource_exists::<Connection>),
        );
    }
}

/// Where the connection to the server stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Said hello, waiting for an answer.
    Joining,
    Joined {
        you: PlayerId,
    },
    Refused(String),
    /// Couldn't join, or the server went away: why.
    Lost(String),
}

/// The connection to a server, while playing on one.
#[derive(Resource)]
pub struct Connection {
    link: Box<dyn Link>,
    pub status: Status,
    /// Everyone on the server but us, by number.
    pub players: BTreeMap<PlayerId, String>,
    /// The looks of every car we've been told of.
    looks: HashMap<PlayerId, Looks>,
    /// The looks last sent of our car, and when its state was (s).
    sent_looks: Option<Looks>,
    sent: f64,
}

impl Connection {
    /// Joins a server over `link` as `name`, playing on `terrain`'s map.
    pub fn new(link: impl Link, name: &str, terrain: &Terrain) -> Self {
        let mut connection = Self {
            link: Box::new(link),
            status: Status::Joining,
            players: BTreeMap::new(),
            looks: HashMap::new(),
            sent_looks: None,
            sent: f64::NEG_INFINITY,
        };
        connection.say(&ToServer::Hello {
            protocol: PROTOCOL,
            map: terrain.fingerprint,
            name: name.to_string(),
        });
        connection
    }

    fn say(&mut self, message: &ToServer) {
        self.link.send(message.reliable(), encode(message));
    }

    /// Leaves the server.
    pub fn close(&mut self) {
        self.link.close();
    }
}

/// The server's clock less ours, for the traffic (like `Proxy::offset`).
#[derive(Resource, Default)]
struct TrafficClock {
    offset: Option<f64>,
}

/// A traffic car on the server, shown here.
#[derive(Component)]
pub struct ServerTraffic {
    pub id: u32,
    states: VecDeque<TrafficState>,
    heard: f64,
    /// Where it is by now, as a body to run into.
    pub body: RigidBody,
    drawn: Drawn,
}

/// Another player's car.
#[derive(Component)]
pub struct Proxy {
    pub id: PlayerId,
    /// The latest states, oldest first.
    states: VecDeque<CarState>,
    /// Our clock less theirs, as low as it's been seen (the quickest a state came over),
    /// creeping up so a slower route is followed (s).
    offset: f64,
    /// When it was last heard of (our clock, s).
    heard: f64,
    /// Where it is by now, as a body to run into (moving, but never pushed).
    pub body: Option<RigidBody>,
    drawn: Drawn,
}

fn now(time: &Time<Real>) -> f64 {
    time.elapsed_secs_f64()
}

fn receive(
    mut commands: Commands,
    mut connection: ResMut<Connection>,
    mut traffic_clock: ResMut<TrafficClock>,
    time: Res<Time<Real>>,
    mut proxies: Query<(Entity, &mut Proxy)>,
    mut traffic_cars: Query<&mut ServerTraffic>,
) {
    let now = now(&time);
    let connection = &mut *connection;
    while let Some(bytes) = connection.link.receive() {
        let Some(message) = decode::<ToClient>(&bytes) else {
            if bytes.first() == Some(&ToClient::LOOKS) {
                warn!(
                    "Another player drives a car this game doesn't know (a newer version): \
                     it isn't shown"
                );
            } else {
                warn!("The server sent something unreadable");
            }
            continue;
        };
        match message {
            ToClient::Welcome { you } => {
                info!("Joined the server as player {you}");
                connection.status = Status::Joined { you };
            }
            ToClient::Refused(why) => {
                warn!("The server refused: {why}");
                connection.status = Status::Refused(why);
            }
            ToClient::Joined { id, name } => {
                connection.players.insert(id, name);
            }
            ToClient::Left { id } => {
                connection.players.remove(&id);
                connection.looks.remove(&id);
                for (entity, proxy) in &proxies {
                    if proxy.id == id {
                        commands.entity(entity).despawn();
                    }
                }
            }
            ToClient::Looks { id, looks } => {
                for (entity, proxy) in &proxies {
                    if proxy.id == id {
                        commands.entity(entity).insert((*looks).clone());
                    }
                }
                connection.looks.insert(id, *looks);
            }
            ToClient::Traffic(cars) => {
                for state in cars {
                    let sample = now - state.time;
                    traffic_clock.offset = Some(match traffic_clock.offset {
                        Some(offset) if sample >= offset => offset + 0.01 * (sample - offset),
                        _ => sample,
                    });
                    let found = traffic_cars.iter_mut().find(|car| car.id == state.id);
                    if let Some(mut car) = found {
                        car.heard = now;
                        if car.states.back().is_none_or(|last| state.time > last.time) {
                            car.states.push_back(state);
                            if car.states.len() > KEPT {
                                car.states.pop_front();
                            }
                        }
                        continue;
                    }
                    let car = TrafficCar::shown(state.chassis, state.paint);
                    commands.spawn((
                        ServerTraffic {
                            id: state.id,
                            states: VecDeque::from([state]),
                            heard: now,
                            body: car.body.clone(),
                            drawn: Drawn::default(),
                        },
                        Transform::from_translation(state.position).with_rotation(state.rotation),
                        car,
                    ));
                }
            }
            ToClient::States(states) => {
                for (id, state) in states {
                    let sample = now - state.time;
                    let found = proxies.iter_mut().find(|(_, proxy)| proxy.id == id);
                    if let Some((_, mut proxy)) = found {
                        proxy.heard = now;
                        proxy.offset = if sample < proxy.offset {
                            sample
                        } else {
                            proxy.offset + 0.01 * (sample - proxy.offset)
                        };
                        if proxy
                            .states
                            .back()
                            .is_none_or(|last| state.time > last.time)
                        {
                            proxy.states.push_back(state);
                            if proxy.states.len() > KEPT {
                                proxy.states.pop_front();
                            }
                        }
                        continue;
                    }
                    // A car new to us (or back in range).
                    let mut entity = commands.spawn((
                        Proxy {
                            id,
                            states: VecDeque::from([state]),
                            offset: sample,
                            heard: now,
                            body: None,
                            drawn: Drawn::default(),
                        },
                        Transform::from_translation(state.position).with_rotation(state.rotation),
                        state.motion,
                    ));
                    if let Some(looks) = connection.looks.get(&id) {
                        entity.insert(looks.clone());
                    }
                }
            }
        }
    }
    let ended = matches!(connection.status, Status::Lost(_) | Status::Refused(_));
    if !connection.link.is_open() && !ended {
        let why = (connection.link.problem())
            .unwrap_or_else(|| "The connection to the server was lost".to_string());
        warn!("{why}");
        connection.status = Status::Lost(why);
        for (entity, _) in &proxies {
            commands.entity(entity).despawn();
        }
    }
}

/// Gives a proxy its car's looks once they're here: a state can overtake them (it comes
/// as a datagram, they come over the stream) and spawn the proxy first.
fn dress_proxies(
    mut commands: Commands,
    connection: Res<Connection>,
    proxies: Query<(Entity, &Proxy), Without<Looks>>,
) {
    for (entity, proxy) in &proxies {
        if let Some(looks) = connection.looks.get(&proxy.id) {
            commands.entity(entity).insert(looks.clone());
        }
    }
}

/// Our car's looks when they change, and its state every so often.
fn send(
    mut connection: ResMut<Connection>,
    time: Res<Time<Real>>,
    active: Res<ActiveCar>,
    cars: Query<(&Car, &Looks, &Motion)>,
) {
    let now = now(&time);
    if !matches!(connection.status, Status::Joined { .. }) {
        return;
    }
    let Some((car, looks, motion)) = cars.iter().find(|(car, ..)| car.index == active.0) else {
        return;
    };
    if connection.sent_looks.as_ref() != Some(looks) {
        connection.sent_looks = Some(looks.clone());
        connection.say(&ToServer::Looks(Box::new(looks.clone())));
    }
    if now - connection.sent >= 1.0 / SEND_RATE {
        connection.sent = now;
        connection.say(&ToServer::State(CarState {
            time: now,
            position: car.body.position,
            rotation: car.body.rotation,
            velocity: car.body.velocity,
            angular_velocity: car.body.angular_velocity,
            motion: *motion,
        }));
    }
}

/// Shows each proxy where it's predicted to be by now, and keeps its body there.
fn move_proxies(
    time: Res<Time<Real>>,
    mut proxies: Query<(&mut Proxy, &mut Transform, &mut Motion, Option<&Looks>)>,
) {
    let now = now(&time);
    let dt = time.delta_secs();
    for (mut proxy, mut transform, mut motion, looks) in &mut proxies {
        let proxy = &mut *proxy;
        let (by_now, ahead) = predict(&proxy.states, now - proxy.offset);
        (transform.translation, transform.rotation) = proxy.drawn.follow(&by_now, dt);
        // The wheels roll on as fast as they did between the last two states.
        let (before, latest) = (proxy.states.front().unwrap(), proxy.states.back().unwrap());
        *motion = latest.motion;
        if latest.time > before.time {
            let between = (latest.time - before.time) as f32;
            for (wheel, was) in motion.wheels.iter_mut().zip(&before.motion.wheels) {
                wheel.spin += (wheel.spin - was.spin) / between * ahead;
            }
        }
        proxy.body = looks.map(|looks| {
            let mut body =
                RigidBody::new(by_now.position, by_now.rotation, looks.mass, looks.inertia);
            body.velocity = by_now.velocity;
            body.angular_velocity = by_now.angular_velocity;
            body
        });
    }
}

/// How a car moves at some moment.
#[derive(Clone, Copy, Debug)]
struct Kinematics {
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    angular_velocity: Vec3,
}

impl Kinematics {
    /// Carried on `dt` seconds: turning as it turns, its velocity turning with it and
    /// changing by `acceleration` (in the car's frame) until it stops.
    fn ahead(mut self, mut acceleration: Vec3, dt: f32) -> Self {
        let mut local = self.rotation.inverse() * self.velocity;
        let steps = (dt / STEP).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as usize {
            let next = local + acceleration * h;
            if next.dot(local) < 0.0 {
                // Braked to a stop (not into reversing).
                local = Vec3::ZERO;
                acceleration = Vec3::ZERO;
                self.angular_velocity = Vec3::ZERO;
            } else {
                local = next;
            }
            self.rotation =
                (Quat::from_scaled_axis(self.angular_velocity * h) * self.rotation).normalize();
            self.position += self.rotation * local * h;
        }
        self.velocity = self.rotation * local;
        self
    }
}

/// A state of a car, to carry on from.
trait Snapshot: Copy {
    fn time(&self) -> f64;
    fn kinematics(&self) -> Kinematics;
}

impl Snapshot for CarState {
    fn time(&self) -> f64 {
        self.time
    }

    fn kinematics(&self) -> Kinematics {
        Kinematics {
            position: self.position,
            rotation: self.rotation,
            velocity: self.velocity,
            angular_velocity: self.angular_velocity,
        }
    }
}

impl Snapshot for TrafficState {
    fn time(&self) -> f64 {
        self.time
    }

    fn kinematics(&self) -> Kinematics {
        Kinematics {
            position: self.position,
            rotation: self.rotation,
            velocity: self.velocity,
            angular_velocity: self.angular_velocity,
        }
    }
}

/// The acceleration from `before` to `after`, in the car's frame (so a steady turn has
/// none: the velocity turns with the car), along and across it only (not the bumps).
fn acceleration<S: Snapshot>(before: &S, after: &S) -> Vec3 {
    let between = after.time() - before.time();
    if !(0.001..=1.0).contains(&between) {
        return Vec3::ZERO;
    }
    let local = |k: Kinematics| k.rotation.inverse() * k.velocity;
    let change = (local(after.kinematics()) - local(before.kinematics())) / between as f32;
    Vec3::new(change.x, 0.0, change.z).clamp_length_max(MOST_ACCELERATION)
}

/// Where a car is at `time` (its clock): its latest state carried on with the
/// acceleration between its last two (up to `PREDICT`), and how long after that state.
fn predict<S: Snapshot>(states: &VecDeque<S>, time: f64) -> (Kinematics, f32) {
    let latest = states.back().unwrap();
    let acceleration = match states.len() {
        n if n >= 2 => acceleration(&states[n - 2], latest),
        _ => Vec3::ZERO,
    };
    let ahead = (time - latest.time()).clamp(0.0, PREDICT) as f32;
    (latest.kinematics().ahead(acceleration, ahead), ahead)
}

/// Where a car is drawn: moving as it's predicted to and closing on where that is, so a
/// new state that disagrees with the last doesn't make it jump.
#[derive(Default)]
struct Drawn(Option<(Vec3, Quat)>);

impl Drawn {
    /// Where to draw it now, `dt` after last time, with it predicted to be `at`.
    fn follow(&mut self, at: &Kinematics, dt: f32) -> (Vec3, Quat) {
        let pose = match self.0 {
            Some((position, rotation)) if position.distance(at.position) < SNAP => {
                let position = position + at.velocity * dt;
                let rotation = Quat::from_scaled_axis(at.angular_velocity * dt) * rotation;
                let closing = 1.0 - (-dt / SMOOTHING).exp();
                (
                    position.lerp(at.position, closing),
                    rotation.slerp(at.rotation, closing).normalize(),
                )
            }
            _ => (at.position, at.rotation),
        };
        self.0 = Some(pose);
        pose
    }
}

/// Proxies not heard of for a while are out of range.
fn remove_proxies(
    mut commands: Commands,
    time: Res<Time<Real>>,
    proxies: Query<(Entity, &Proxy)>,
    traffic_cars: Query<(Entity, &ServerTraffic)>,
) {
    let now = now(&time);
    for (entity, proxy) in &proxies {
        if now - proxy.heard > GONE {
            commands.entity(entity).despawn();
        }
    }
    for (entity, car) in &traffic_cars {
        if now - car.heard > GONE {
            commands.entity(entity).despawn();
        }
    }
}

/// On a server the traffic is the server's (and what was there goes); off it, our own
/// again (and the server's goes).
fn traffic_source(
    mut commands: Commands,
    connection: Option<Res<Connection>>,
    mut source: ResMut<traffic::Source>,
    mut clock: ResMut<TrafficClock>,
    cars: Query<Entity, With<TrafficCar>>,
) {
    let wanted = match connection.map(|c| c.status.clone()) {
        Some(Status::Joined { .. }) => traffic::Source::Server,
        _ => traffic::Source::Own,
    };
    if *source != wanted {
        *source = wanted;
        clock.offset = None;
        for entity in &cars {
            commands.entity(entity).despawn();
        }
    }
}

/// Shows each of the server's traffic cars where it's predicted to be by now, and keeps
/// its body there.
fn move_traffic(
    time: Res<Time<Real>>,
    clock: Res<TrafficClock>,
    mut cars: Query<(&mut ServerTraffic, &mut TrafficCar)>,
) {
    let Some(offset) = clock.offset else {
        return;
    };
    let now = now(&time);
    let dt = time.delta_secs();
    for (mut shown, mut car) in &mut cars {
        let shown = &mut *shown;
        let (by_now, ahead) = predict(&shown.states, now - offset);
        (car.body.position, car.body.rotation) = shown.drawn.follow(&by_now, dt);
        car.body.velocity = by_now.velocity;
        car.body.angular_velocity = by_now.angular_velocity;
        let latest = shown.states.back().unwrap();
        let forward = latest.rotation * Vec3::NEG_Z;
        car.turn_wheels(
            latest.rolled + latest.velocity.dot(forward) * ahead,
            latest.steer,
        );
        shown.body.position = by_now.position;
        shown.body.rotation = by_now.rotation;
        shown.body.velocity = by_now.velocity;
        shown.body.angular_velocity = by_now.angular_velocity;
    }
}

/// Our cars push themselves off the proxies they run into (the proxies aren't moved: the
/// other player's game pushes theirs off ours).
fn push_off_proxies(
    time: Res<Time>,
    mut cars: Query<&mut Car>,
    proxies: Query<(&Proxy, &Looks)>,
    traffic_cars: Query<(&ServerTraffic, &TrafficCar)>,
) {
    let dt = time.delta_secs();
    let others: Vec<(RigidBody, Obb)> = (proxies.iter())
        .filter_map(|(proxy, looks)| Some((proxy.body.clone()?, looks.hull)))
        .chain((traffic_cars.iter()).map(|(shown, car)| (shown.body.clone(), car.hull)))
        .collect();
    for mut car in &mut cars {
        let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
        for (body, other) in &others {
            let mut body = body.clone();
            let other = other.placed(body.position, body.rotation);
            let reach = hull.radius() + other.radius();
            if hull.center.distance_squared(other.center) > reach * reach {
                continue;
            }
            if let Some(contact) = hull.contact(&other) {
                let impulse =
                    collision::resolve(&mut car.body, Some(&mut body), &[contact], OBSTACLE, dt);
                car.knocks += impulse / car.body.mass;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state of a car on a circle of `radius` around the origin at `speed`, braking at
    /// `braking`, `time` into it (and its speed then).
    fn on_circle(radius: f32, speed: f32, braking: f32, time: f64) -> TrafficState {
        let t = time as f32;
        let speed = speed - braking * t;
        let angle = (speed + braking * t / 2.0) * t / radius;
        let yaw_rate = speed / radius;
        let rotation = Quat::from_rotation_y(angle);
        // Starts at (radius, 0, 0) heading -z, turning left (around +y).
        let position = Vec3::new(radius * angle.cos(), 0.0, -radius * angle.sin());
        TrafficState {
            id: 0,
            chassis: 0,
            paint: 0,
            time,
            position,
            rotation,
            velocity: rotation * Vec3::new(0.0, 0.0, -speed),
            angular_velocity: Vec3::new(0.0, yaw_rate, 0.0),
            rolled: 0.0,
            steer: 0.0,
        }
    }

    #[test]
    fn a_car_braking_in_a_bend_is_predicted_along_it() {
        // 20 m/s on a 60 m bend, braking at 4 m/s², states 3 a second (far away).
        let states = VecDeque::from([
            on_circle(60.0, 20.0, 4.0, 0.0),
            on_circle(60.0, 20.0, 4.0, 1.0 / 3.0),
        ]);
        for ahead in [0.1, 0.3, 0.6] {
            let time = 1.0 / 3.0 + ahead;
            let (predicted, _) = predict(&states, time);
            let truth = on_circle(60.0, 20.0, 4.0, time);
            let off = predicted.position.distance(truth.position);
            // Straight on at the same speed, as before, for comparison.
            let latest = states[1];
            let straight = latest.position + latest.velocity * ahead as f32;
            println!(
                "{ahead} s ahead: {off:.3} m off (straight on: {:.2} m)",
                straight.distance(truth.position)
            );
            assert!(off < 0.1, "{off}");
        }
    }

    #[test]
    fn a_car_braking_to_a_stop_stays_stopped() {
        let state = |time: f64, speed: f32| TrafficState {
            id: 0,
            chassis: 0,
            paint: 0,
            time,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::new(0.0, 0.0, -speed),
            angular_velocity: Vec3::new(0.0, 0.2, 0.0),
            rolled: 0.0,
            steer: 0.0,
        };
        // 2 m/s, braking at 6 m/s²: stopped after 1/3 s, 0.33 m on.
        let states = VecDeque::from([state(0.0, 3.0), state(1.0 / 6.0, 2.0)]);
        let (stopped, _) = predict(&states, 1.0 / 6.0 + PREDICT);
        assert_eq!(stopped.velocity, Vec3::ZERO);
        assert_eq!(stopped.angular_velocity, Vec3::ZERO);
        let travelled = stopped.position.length();
        assert!((0.3..0.36).contains(&travelled), "{travelled}");
    }

    #[test]
    fn a_car_drawn_closes_on_a_correction_without_jumping() {
        let mut drawn = Drawn::default();
        let mut at = Kinematics {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::new(0.0, 0.0, -10.0),
            angular_velocity: Vec3::ZERO,
        };
        let dt = 1.0 / 60.0;
        drawn.follow(&at, dt);
        // A new state puts it a metre further on.
        at.position.z -= 1.0;
        let mut was = Vec3::ZERO;
        for frame in 1..=30 {
            at.position += at.velocity * dt;
            let (position, _) = drawn.follow(&at, dt);
            let step = was.distance(position);
            assert!(step < 10.0 * dt + 0.2, "frame {frame}: moved {step} m");
            was = position;
        }
        // Half a second on (five time constants), it's there.
        assert!(
            was.distance(at.position) < 0.01,
            "{}",
            was.distance(at.position)
        );
    }
}
