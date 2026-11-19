//! A player's side: say hello, send the car being driven (its looks when they change, its
//! state `SEND_RATE` times a second), and show the cars of the players nearby as proxies:
//! entities with their `Looks` and `Motion` (so they're drawn and heard like any car),
//! moved along their states a little in the past (`DELAY`, so there's always a state on
//! either side to interpolate between). For contacts, a proxy is where its last state
//! says it is by now. A car only pushes itself off a proxy; the other player's game does
//! the same on their side. On a server the traffic is the server's: its cars near us are
//! shown the same way (`ServerTraffic`), and our own traffic stops.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::*;

use super::SEND_RATE;
use super::link::Link;
use super::protocol::{CarState, PROTOCOL, PlayerId, ToClient, ToServer, TrafficState};
use super::wire::{decode, encode};
use crate::car::{ActiveCar, Car};
use crate::looks::{Looks, Motion, WheelMotion};
use crate::physics::collision::Obb;
use crate::physics::collision::{self, OBSTACLE};
use crate::physics::rigid_body::RigidBody;
use crate::terrain::Terrain;
use crate::traffic::{self, TrafficCar};

/// How far behind its latest state a proxy is shown (s): more than the time between two
/// states, so there's usually one either side.
const DELAY: f64 = 0.1;
/// How far a proxy is carried on past its latest state when the next is late (s).
const EXTRAPOLATE: f64 = 0.25;
/// A proxy not heard of for this long is gone (out of range) (s).
const GONE: f64 = 2.0;
/// States kept per proxy.
const KEPT: usize = 16;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (receive, send, (move_proxies, remove_proxies))
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

/// Shows each proxy where its states say it was `DELAY` ago, and keeps its body where it
/// is by now.
fn move_proxies(
    time: Res<Time<Real>>,
    mut proxies: Query<(&mut Proxy, &mut Transform, &mut Motion, Option<&Looks>)>,
) {
    let now = now(&time);
    for (mut proxy, mut transform, mut motion, looks) in &mut proxies {
        let shown = now - proxy.offset - DELAY;
        let state = sample(&proxy.states, shown);
        transform.translation = state.position;
        transform.rotation = state.rotation;
        *motion = state.motion;
        let latest = *proxy.states.back().unwrap();
        let ahead = (now - proxy.offset - latest.time).clamp(0.0, EXTRAPOLATE) as f32;
        let by_now = latest.ahead(ahead);
        proxy.body = looks.map(|looks| {
            let mut body =
                RigidBody::new(by_now.position, by_now.rotation, looks.mass, looks.inertia);
            body.velocity = by_now.velocity;
            body.angular_velocity = by_now.angular_velocity;
            body
        });
    }
}

/// A state of something moving, to show it between states.
trait Snapshot: Copy {
    fn time(&self) -> f64;
    /// `t` of the way from `a` to `b`, at `time`.
    fn between(a: &Self, b: &Self, t: f32, time: f64) -> Self;
    /// Carried on `dt` seconds.
    fn ahead(&self, dt: f32) -> Self;
}

impl Snapshot for CarState {
    fn time(&self) -> f64 {
        self.time
    }

    fn between(a: &Self, b: &Self, t: f32, time: f64) -> Self {
        let wheel = |a: &WheelMotion, b: &WheelMotion| WheelMotion {
            theta: a.theta.lerp(b.theta, t),
            steer: a.steer.lerp(b.steer, t),
            spin: a.spin.lerp(b.spin, t),
        };
        let mut motion = b.motion;
        for k in 0..4 {
            motion.wheels[k] = wheel(&a.motion.wheels[k], &b.motion.wheels[k]);
        }
        let (ea, eb) = (&a.motion.engine, &b.motion.engine);
        motion.engine.rpm = ea.rpm.lerp(eb.rpm, t);
        motion.engine.load = ea.load.lerp(eb.load, t);
        motion.engine.turbo_speed = ea.turbo_speed.lerp(eb.turbo_speed, t);
        CarState {
            time,
            position: a.position.lerp(b.position, t),
            rotation: a.rotation.slerp(b.rotation, t),
            velocity: a.velocity.lerp(b.velocity, t),
            angular_velocity: a.angular_velocity.lerp(b.angular_velocity, t),
            motion,
        }
    }

    fn ahead(&self, dt: f32) -> Self {
        CarState {
            time: self.time + f64::from(dt),
            position: self.position + self.velocity * dt,
            rotation: (Quat::from_scaled_axis(self.angular_velocity * dt) * self.rotation)
                .normalize(),
            ..*self
        }
    }
}

impl Snapshot for TrafficState {
    fn time(&self) -> f64 {
        self.time
    }

    fn between(a: &Self, b: &Self, t: f32, time: f64) -> Self {
        TrafficState {
            time,
            position: a.position.lerp(b.position, t),
            rotation: a.rotation.slerp(b.rotation, t),
            velocity: a.velocity.lerp(b.velocity, t),
            angular_velocity: a.angular_velocity.lerp(b.angular_velocity, t),
            rolled: a.rolled.lerp(b.rolled, t),
            steer: a.steer.lerp(b.steer, t),
            ..*b
        }
    }

    fn ahead(&self, dt: f32) -> Self {
        let forward = self.rotation * Vec3::NEG_Z;
        TrafficState {
            time: self.time + f64::from(dt),
            position: self.position + self.velocity * dt,
            rotation: (Quat::from_scaled_axis(self.angular_velocity * dt) * self.rotation)
                .normalize(),
            rolled: self.rolled + self.velocity.dot(forward) * dt,
            ..*self
        }
    }
}

/// The state at `time` (its clock): between the two states around it, or carried on
/// from the last (a little, then held).
fn sample<S: Snapshot>(states: &VecDeque<S>, time: f64) -> S {
    let last = *states.back().unwrap();
    if time >= last.time() {
        return last.ahead((time - last.time()).min(EXTRAPOLATE) as f32);
    }
    let Some(i) = states.iter().position(|s| s.time() > time) else {
        return last;
    };
    if i == 0 {
        return states[0];
    }
    let (a, b) = (&states[i - 1], &states[i]);
    let t = ((time - a.time()) / (b.time() - a.time())) as f32;
    S::between(a, b, t, time)
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

/// Shows each of the server's traffic cars where it was `DELAY` ago, and keeps its body
/// where it is by now.
fn move_traffic(
    time: Res<Time<Real>>,
    clock: Res<TrafficClock>,
    mut cars: Query<(&mut ServerTraffic, &mut TrafficCar)>,
) {
    let Some(offset) = clock.offset else {
        return;
    };
    let now = now(&time);
    for (mut shown, mut car) in &mut cars {
        let state = sample(&shown.states, now - offset - DELAY);
        car.body.position = state.position;
        car.body.rotation = state.rotation;
        car.body.velocity = state.velocity;
        car.body.angular_velocity = state.angular_velocity;
        car.turn_wheels(state.rolled, state.steer);
        let latest = *shown.states.back().unwrap();
        let by_now = latest.ahead((now - offset - latest.time).clamp(0.0, EXTRAPOLATE) as f32);
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
