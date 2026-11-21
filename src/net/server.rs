//! The server's side: players join, their cars' states go to the players near them (the
//! nearer, the more often), their looks before that and after every change. A player
//! who falls silent is frozen for the others (and traffic stops coming for them), and
//! dropped after a while. Each player is an entity with a `Peer`, and a `traffic::Anchor`
//! and `traffic::Visitor` while their car is heard from, so traffic drives around them,
//! keeps its distance and crashes into them. Each player is sent the traffic near them
//! like the other players' cars.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use super::link::{Link, Listener};
use super::protocol::{CarState, PROTOCOL, PlayerId, ToClient, ToServer, TrafficState};
use super::wire::{decode, encode};
use super::{NEAR, RATES, SEND_RATE};
use crate::looks::Looks;
use crate::physics::rigid_body::RigidBody;
use crate::terrain::Terrain;
use crate::traffic::{Anchor, TrafficCar, Visitor};

/// A player not heard from for this long is frozen for the others, and for this long
/// dropped (s).
const FROZEN: f64 = 1.0;
const TIMEOUT: f64 = 30.0;
/// The grid players are sorted into for finding who's near whom (m).
const CELL: f32 = 128.0;
/// The most states in one message (a datagram holds about 1200 bytes).
const STATES_PER_MESSAGE: usize = 8;
const TRAFFIC_PER_MESSAGE: usize = 14;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Presence>().add_systems(
            Update,
            (accept, receive, send)
                .chain()
                .run_if(resource_exists::<Server>),
        );
    }
}

/// A player joined or left (for telling the server's owner, e.g. `bin/server.rs`'s
/// Telegram bot).
#[derive(Message, Clone, Debug)]
pub struct Presence {
    pub name: String,
    /// Joined, else left.
    pub joined: bool,
    /// How many players there are now.
    pub players: usize,
}

/// The server: where players come from.
#[derive(Resource)]
pub struct Server {
    listener: Box<dyn Listener>,
    next_id: PlayerId,
    /// The most players at once.
    max_players: usize,
    /// When states were last sent (s).
    sent: f64,
}

impl Server {
    pub fn new(listener: impl Listener) -> Self {
        Self {
            listener: Box::new(listener),
            next_id: 1,
            max_players: 32,
            sent: 0.0,
        }
    }

    pub fn with_max_players(self, max_players: usize) -> Self {
        Self {
            max_players,
            ..self
        }
    }
}

/// A player connected to the server.
#[derive(Component)]
pub struct Peer {
    pub id: PlayerId,
    link: Box<dyn Link>,
    /// Their name, once they've said hello.
    pub name: Option<String>,
    /// When they were last heard from (s).
    heard: f64,
    /// Their car's latest state and looks (with a revision counting the changes).
    pub state: Option<CarState>,
    pub looks: Option<Looks>,
    revision: u32,
    /// What they've been told of each car near them: its looks' revision, and when its
    /// next state is due (s).
    told: HashMap<PlayerId, (u32, f64)>,
    /// When each traffic car's next state is due for them (s).
    told_traffic: HashMap<u32, f64>,
}

impl Peer {
    fn tell(&mut self, message: &ToClient) {
        self.link.send(message.reliable(), encode(message));
    }
}

fn now(time: &Time<Real>) -> f64 {
    time.elapsed_secs_f64()
}

fn accept(mut commands: Commands, mut server: ResMut<Server>, time: Res<Time<Real>>) {
    while let Some(link) = server.listener.accept() {
        let id = server.next_id;
        server.next_id += 1;
        commands.spawn(Peer {
            id,
            link,
            name: None,
            heard: now(&time),
            state: None,
            looks: None,
            revision: 0,
            told: HashMap::new(),
            told_traffic: HashMap::new(),
        });
    }
}

/// Reads what every player said, and lets go of the ones who left or fell silent.
fn receive(
    mut commands: Commands,
    server: Res<Server>,
    terrain: Res<Terrain>,
    time: Res<Time<Real>>,
    mut peers: Query<(Entity, &mut Peer, Option<&mut Transform>)>,
    mut presence: MessageWriter<Presence>,
) {
    let now = now(&time);
    let mut present = peers.iter().filter(|(_, p, _)| p.name.is_some()).count();
    // What everyone else has to be told afterwards.
    let mut joined = Vec::new();
    let mut left = Vec::new();
    for (entity, mut peer, mut transform) in &mut peers {
        while let Some(bytes) = peer.link.receive() {
            let Some(message) = decode::<ToServer>(&bytes) else {
                // Looks out of our catalogues come from a newer game (a car or part the
                // server's version doesn't have yet): say so.
                let why = if bytes.first() == Some(&ToServer::LOOKS) {
                    "The server doesn't know your car: it runs an older version of the game \
                     and needs updating"
                } else {
                    "Your game sent something the server couldn't read"
                };
                warn!("Player {}: {why}; dropped", peer.id);
                peer.tell(&ToClient::Refused(why.to_string()));
                peer.link.close();
                break;
            };
            peer.heard = now;
            match message {
                ToServer::Hello {
                    protocol,
                    map,
                    name,
                } if peer.name.is_none() => {
                    let refusal = if protocol != PROTOCOL {
                        Some(format!(
                            "The server runs version {PROTOCOL} of the game's protocol, \
                             this game version {protocol}"
                        ))
                    } else if map != terrain.fingerprint {
                        Some("The server has another map".to_string())
                    } else if present >= server.max_players {
                        Some("The server is full".to_string())
                    } else {
                        None
                    };
                    if let Some(why) = refusal {
                        peer.tell(&ToClient::Refused(why));
                        peer.link.close();
                        break;
                    }
                    let name = name.trim().to_string();
                    info!("{name} joined (player {})", peer.id);
                    let id = peer.id;
                    peer.tell(&ToClient::Welcome { you: id });
                    joined.push((id, name.clone()));
                    present += 1;
                    peer.name = Some(name);
                }
                // Nothing else before hello.
                _ if peer.name.is_none() => {}
                ToServer::Hello { .. } => {}
                ToServer::Looks(looks) => {
                    peer.looks = Some(*looks);
                    peer.revision += 1;
                }
                ToServer::State(state) => {
                    if peer.state.is_none_or(|old| state.time > old.time) {
                        peer.state = Some(state);
                        if let Some(looks) = &peer.looks {
                            let mut body = RigidBody::new(
                                state.position,
                                state.rotation,
                                looks.mass,
                                looks.inertia,
                            );
                            body.velocity = state.velocity;
                            body.angular_velocity = state.angular_velocity;
                            commands.entity(entity).insert(Visitor {
                                body,
                                hull: looks.hull,
                            });
                        }
                        let at = Transform::from_translation(state.position)
                            .with_rotation(state.rotation);
                        match transform.as_deref_mut() {
                            Some(t) => *t = at,
                            None => {
                                commands.entity(entity).insert(at);
                            }
                        }
                    }
                }
            }
        }
        let silent = now - peer.heard;
        if !peer.link.is_open() || silent > TIMEOUT {
            if let Some(name) = &peer.name {
                info!("{name} left (player {})", peer.id);
                left.push((peer.id, name.clone()));
            }
            commands.entity(entity).despawn();
        } else if silent > FROZEN || peer.state.is_none() {
            commands.entity(entity).remove::<(Anchor, Visitor)>();
        } else {
            commands.entity(entity).insert(Anchor);
        }
    }

    // Everyone hears of who joined and who left; the new ones of everyone there.
    for (id, name) in &joined {
        for (_, mut peer, _) in &mut peers {
            if peer.name.is_none() || peer.id == *id {
                continue;
            }
            peer.tell(&ToClient::Joined {
                id: *id,
                name: name.clone(),
            });
        }
    }
    let present: Vec<(PlayerId, String)> = (peers.iter())
        .filter_map(|(_, peer, _)| Some((peer.id, peer.name.clone()?)))
        .collect();
    for (id, _) in &joined {
        for (_, mut peer, _) in &mut peers {
            if peer.id != *id {
                continue;
            }
            for (other, name) in present.iter().filter(|(other, _)| other != id) {
                peer.tell(&ToClient::Joined {
                    id: *other,
                    name: name.clone(),
                });
            }
        }
    }
    // The ones who left are still there until the commands run.
    let players = present.len() - left.len();
    for (_, name) in &joined {
        presence.write(Presence {
            name: name.clone(),
            joined: true,
            players,
        });
    }
    for (id, name) in left {
        presence.write(Presence {
            name,
            joined: false,
            players,
        });
        for (_, mut peer, _) in &mut peers {
            if peer.name.is_some() && peer.id != id {
                peer.told.remove(&id);
                peer.tell(&ToClient::Left { id });
            }
        }
    }
}

/// Sends each player the states of the cars near them (with their looks when they're
/// new to them or changed), each as often as its distance asks for.
fn send(
    mut server: ResMut<Server>,
    time: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    mut peers: Query<&mut Peer>,
    traffic: Query<&TrafficCar>,
) {
    let now = now(&time);
    if now - server.sent < 1.0 / SEND_RATE {
        return;
    }
    server.sent = now;

    // The cars there are to see: heard from lately, with looks.
    struct Car {
        id: PlayerId,
        state: CarState,
        looks: Looks,
        revision: u32,
    }
    let cars: Vec<Car> = peers
        .iter()
        .filter(|peer| now - peer.heard <= FROZEN)
        .filter_map(|peer| {
            Some(Car {
                id: peer.id,
                state: peer.state?,
                looks: peer.looks.clone()?,
                revision: peer.revision,
            })
        })
        .collect();
    let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, car) in cars.iter().enumerate() {
        grid.entry(cell(car.state.position)).or_default().push(i);
    }
    let reach = (NEAR / CELL).ceil() as i32;
    let stamp = super::stamp(&time, &fixed);
    let traffic: Vec<TrafficState> = traffic
        .iter()
        .map(|car| {
            let (rolled, steer) = car.wheels_turned();
            TrafficState {
                id: car.id,
                chassis: car.chassis,
                paint: car.paint,
                time: stamp,
                position: car.body.position,
                rotation: car.body.rotation,
                velocity: car.body.velocity,
                angular_velocity: car.body.angular_velocity,
                rolled,
                steer,
            }
        })
        .collect();
    let present: HashSet<u32> = traffic.iter().map(|car| car.id).collect();
    // How often something this far away is due, and when next.
    let due_after = |distance: f32| {
        let rate = RATES
            .iter()
            .find(|&&(within, _)| distance <= within)
            .map_or(RATES[RATES.len() - 1].1, |&(_, rate)| rate);
        // A little early, so a rate that's the send rate keeps up.
        now + 1.0 / rate - 0.5 / SEND_RATE
    };

    for mut peer in &mut peers {
        let Some(own) = peer.state.filter(|_| peer.name.is_some()) else {
            continue;
        };
        let (cx, cz) = cell(own.position);
        let mut states = Vec::new();
        for x in cx - reach..=cx + reach {
            for z in cz - reach..=cz + reach {
                for &i in grid.get(&(x, z)).into_iter().flatten() {
                    let car = &cars[i];
                    let distance = car.state.position.distance(own.position);
                    if car.id == peer.id || distance > NEAR {
                        continue;
                    }
                    let (revision, due) = peer.told.get(&car.id).copied().unwrap_or((0, 0.0));
                    if revision != car.revision {
                        peer.tell(&ToClient::Looks {
                            id: car.id,
                            looks: Box::new(car.looks.clone()),
                        });
                    }
                    let due = if now >= due {
                        states.push((car.id, car.state));
                        due_after(distance)
                    } else {
                        due
                    };
                    peer.told.insert(car.id, (car.revision, due));
                }
            }
        }
        for chunk in states.chunks(STATES_PER_MESSAGE) {
            peer.tell(&ToClient::States(chunk.to_vec()));
        }

        // The traffic near them (there's little of it: no grid needed).
        let mut cars = Vec::new();
        let peer = &mut *peer;
        peer.told_traffic.retain(|id, _| present.contains(id));
        for car in &traffic {
            let distance = car.position.distance(own.position);
            if distance > NEAR {
                peer.told_traffic.remove(&car.id);
                continue;
            }
            let due = peer.told_traffic.entry(car.id).or_insert(0.0);
            if now >= *due {
                cars.push(*car);
                *due = due_after(distance);
            }
        }
        for chunk in cars.chunks(TRAFFIC_PER_MESSAGE) {
            peer.tell(&ToClient::Traffic(chunk.to_vec()));
        }
    }
}
