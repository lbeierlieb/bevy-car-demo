//! A player's side: say hello, send the car being driven (its looks when they change, its
//! state `SEND_RATE` times a second), and show the cars of the players nearby as proxies:
//! entities with their `Looks` and `Motion` (so they're drawn and heard like any car),
//! moved along their states a little in the past (`DELAY`, so there's always a state on
//! either side to interpolate between). For contacts, a proxy is where its last state
//! says it is by now. A car only pushes itself off a proxy; the other player's game does
//! the same on their side.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bevy::prelude::*;

use super::SEND_RATE;
use super::link::Link;
use super::protocol::{CarState, PROTOCOL, PlayerId, ToClient, ToServer};
use super::wire::{decode, encode};
use crate::car::{ActiveCar, Car};
use crate::looks::{Looks, Motion, WheelMotion};
use crate::physics::collision::{self, OBSTACLE};
use crate::physics::rigid_body::RigidBody;
use crate::terrain::Terrain;

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
    /// The server went away.
    Lost,
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
    time: Res<Time<Real>>,
    mut proxies: Query<(Entity, &mut Proxy)>,
) {
    let now = now(&time);
    let connection = &mut *connection;
    while let Some(bytes) = connection.link.receive() {
        let Some(message) = decode::<ToClient>(&bytes) else {
            warn!("The server sent something unreadable");
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
    if !connection.link.is_open() && connection.status != Status::Lost {
        if !matches!(connection.status, Status::Refused(_)) {
            warn!("Lost the connection to the server");
            connection.status = Status::Lost;
        }
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
        proxy.body = looks.map(|looks| {
            let ahead = (now - proxy.offset - latest.time).clamp(0.0, EXTRAPOLATE) as f32;
            let mut body = RigidBody::new(
                latest.position + latest.velocity * ahead,
                Quat::from_scaled_axis(latest.angular_velocity * ahead) * latest.rotation,
                looks.mass,
                looks.inertia,
            );
            body.velocity = latest.velocity;
            body.angular_velocity = latest.angular_velocity;
            body
        });
    }
}

/// The state at `time` (their clock): between the two states around it, or carried on
/// from the last (a little, then held).
fn sample(states: &VecDeque<CarState>, time: f64) -> CarState {
    let last = *states.back().unwrap();
    if time >= last.time {
        let ahead = (time - last.time).min(EXTRAPOLATE) as f32;
        return CarState {
            position: last.position + last.velocity * ahead,
            rotation: (Quat::from_scaled_axis(last.angular_velocity * ahead) * last.rotation)
                .normalize(),
            ..last
        };
    }
    let Some(i) = states.iter().position(|s| s.time > time) else {
        return last;
    };
    if i == 0 {
        return states[0];
    }
    let (a, b) = (&states[i - 1], &states[i]);
    let t = ((time - a.time) / (b.time - a.time)) as f32;
    let wheel = |a: &WheelMotion, b: &WheelMotion| WheelMotion {
        theta: a.theta.lerp(b.theta, t),
        steer: a.steer.lerp(b.steer, t),
        spin: a.spin.lerp(b.spin, t),
    };
    let mut motion = b.motion;
    for k in 0..4 {
        motion.wheels[k] = wheel(&a.motion.wheels[k], &b.motion.wheels[k]);
    }
    motion.engine.rpm = a.motion.engine.rpm.lerp(b.motion.engine.rpm, t);
    motion.engine.load = a.motion.engine.load.lerp(b.motion.engine.load, t);
    motion.engine.turbo_speed = (a.motion.engine.turbo_speed).lerp(b.motion.engine.turbo_speed, t);
    CarState {
        time,
        position: a.position.lerp(b.position, t),
        rotation: a.rotation.slerp(b.rotation, t),
        velocity: a.velocity.lerp(b.velocity, t),
        angular_velocity: a.angular_velocity.lerp(b.angular_velocity, t),
        motion,
    }
}

/// Proxies not heard of for a while are out of range.
fn remove_proxies(mut commands: Commands, time: Res<Time<Real>>, proxies: Query<(Entity, &Proxy)>) {
    let now = now(&time);
    for (entity, proxy) in &proxies {
        if now - proxy.heard > GONE {
            commands.entity(entity).despawn();
        }
    }
}

/// Our cars push themselves off the proxies they run into (the proxies aren't moved: the
/// other player's game pushes theirs off ours).
fn push_off_proxies(time: Res<Time>, mut cars: Query<&mut Car>, proxies: Query<(&Proxy, &Looks)>) {
    let dt = time.delta_secs();
    for mut car in &mut cars {
        let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
        for (proxy, looks) in &proxies {
            let Some(mut body) = proxy.body.clone() else {
                continue;
            };
            let other = looks.hull.placed(body.position, body.rotation);
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
