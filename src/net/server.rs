//! The server's side: players join, and every car's state goes to the players near it (the
//! nearer, the more often), its looks before that and after every change, and the name
//! it shows (players' cars their player's, traffic none). The traffic's cars and the
//! players' are sent alike, numbered by one counter (`traffic::CarNumbers`). A player
//! who falls silent is frozen for the others (and traffic stops coming for them), and
//! dropped after a while. Each player is an entity with a `Peer`, and a `traffic::Anchor`
//! and `traffic::Visitor` while their car is heard from, so traffic drives around them,
//! keeps its distance and crashes into them. Once a second each player is told where the others
//! are for the map: nobody, the ones near them or everyone, as the server is set
//! (`MapPlayers`), and every two seconds everyone's ping (the connection's round trip).
//! Every state goes out stamped on the server's clock (a player's converted from theirs),
//! so a player has one clock for every car they're sent.
//! With chat on, what a player writes goes to everyone (cleaned up, a few lines at a time
//! at most), and everyone is told in the chat who joined and left.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;

use super::link::{Link, Listener};
use super::protocol::{
    CarId, CarState, MapPlayer, PROTOCOL, PlayerId, SERVER, ToClient, ToServer, clean_chat,
};
use super::wire::{decode, encode};
use super::{ClockOffset, NEAR, RATES, SEND_RATE};
use crate::looks::{Looks, Motion};
use crate::physics::rigid_body::RigidBody;
use crate::shops::Shop;
use crate::terrain::Terrain;
use crate::traffic::{Anchor, CarNumbers, TrafficCar, Visitor};

/// A player not heard from for this long is frozen for the others, and for this long
/// dropped (s).
const FROZEN: f64 = 1.0;
const TIMEOUT: f64 = 30.0;
/// The grid players are sorted into for finding who's near whom (m).
const CELL: f32 = 128.0;
/// The most states in one message (a datagram holds about 1200 bytes).
const STATES_PER_MESSAGE: usize = 8;
const MAP_PER_MESSAGE: usize = 32;
/// How often the other players' places on the map are sent (per second).
const MAP_RATE: f64 = 1.0;
/// The most chat lines a player may write within `CHAT_WINDOW` seconds.
const CHAT_LINES: usize = 5;
const CHAT_WINDOW: f64 = 10.0;
/// How often everyone's ping is sent (per second).
const PING_RATE: f64 = 0.5;
/// How near other players are shown on the map by default (m).
pub const MAP_NEAR: f32 = 200.0;

/// Which other players a player sees on their map (their own car is always there).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapPlayers {
    Nobody,
    /// The ones within this distance (m).
    Near(f32),
    Everyone,
}

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Presence>()
            .init_resource::<CarNumbers>()
            .add_systems(
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
    /// Who's shown on the players' maps.
    pub(super) map: MapPlayers,
    /// Whether players can chat.
    chat: bool,
    /// Whether players' cars show their names.
    name_tags: bool,
    /// The shops on the map, told to every player who joins.
    shops: Vec<Shop>,
    /// When states, and the map's players, were last sent (s).
    sent: f64,
    map_sent: f64,
    pings_sent: f64,
}

impl Server {
    pub fn new(listener: impl Listener) -> Self {
        Self {
            listener: Box::new(listener),
            next_id: 1,
            max_players: 32,
            map: MapPlayers::Near(MAP_NEAR),
            chat: true,
            name_tags: true,
            shops: Vec::new(),
            sent: 0.0,
            map_sent: f64::NEG_INFINITY,
            pings_sent: f64::NEG_INFINITY,
        }
    }

    pub fn with_map(self, map: MapPlayers) -> Self {
        Self { map, ..self }
    }

    pub fn with_chat(self, chat: bool) -> Self {
        Self { chat, ..self }
    }

    pub fn with_shops(self, shops: Vec<Shop>) -> Self {
        Self { shops, ..self }
    }

    pub fn with_name_tags(self, name_tags: bool) -> Self {
        Self { name_tags, ..self }
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
    /// Their car's number (from `CarNumbers`, like the traffic's).
    pub car: CarId,
    link: Box<dyn Link>,
    /// Their name, once they've said hello.
    pub name: Option<String>,
    /// When they were last heard from (s).
    heard: f64,
    /// Their car's latest state and looks (with a revision counting the changes).
    pub state: Option<CarState>,
    pub looks: Option<Looks>,
    /// Our clock less theirs: their states are passed on stamped on ours, so that every
    /// car a player is sent is on one clock.
    clock: ClockOffset,
    revision: u32,
    /// What they've been told of each car near them.
    told: HashMap<CarId, Told>,
    /// When they wrote their latest chat lines (s), the last `CHAT_LINES`.
    chatted: VecDeque<f64>,
}

/// What a player has been told of a car: its looks' revision and the name it shows, and
/// when its next state is due (s).
struct Told {
    revision: u32,
    tag: Option<String>,
    due: f64,
}

impl Peer {
    fn tell(&mut self, message: &ToClient) {
        self.link.send(message.reliable(), encode(message));
    }
}

fn now(time: &Time<Real>) -> f64 {
    time.elapsed_secs_f64()
}

fn accept(
    mut commands: Commands,
    mut server: ResMut<Server>,
    mut numbers: ResMut<CarNumbers>,
    time: Res<Time<Real>>,
) {
    while let Some(link) = server.listener.accept() {
        let id = server.next_id;
        server.next_id += 1;
        commands.spawn(Peer {
            id,
            car: numbers.take(),
            link,
            name: None,
            heard: now(&time),
            state: None,
            looks: None,
            clock: ClockOffset::default(),
            revision: 0,
            told: HashMap::new(),
            chatted: VecDeque::new(),
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
    let mut chat = Vec::new();
    // Who wrote too much at once.
    let mut too_fast = HashSet::new();
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
                    // A web page behind the server is most likely an old copy from the
                    // browser's cache: say first how to get the new one. An old page
                    // shows whatever it's told, so the hint has to come from here.
                    let stale =
                        protocol < PROTOCOL || (protocol == PROTOCOL && map != terrain.fingerprint);
                    let refusal = refusal.map(|why| {
                        if stale && peer.link.browser() {
                            format!(
                                "Press Ctrl+F5 to reload the page without the browser's \
                                 cache: the server runs a newer version of the game.\n\n({why})"
                            )
                        } else {
                            why
                        }
                    });
                    if let Some(why) = refusal {
                        peer.tell(&ToClient::Refused(why));
                        peer.link.close();
                        break;
                    }
                    let name = name.trim().to_string();
                    info!("{name} joined (player {})", peer.id);
                    let id = peer.id;
                    peer.tell(&ToClient::Welcome {
                        you: id,
                        chat: server.chat,
                    });
                    peer.tell(&ToClient::Shops(server.shops.clone()));
                    joined.push((id, name.clone()));
                    present += 1;
                    peer.name = Some(name);
                }
                // Nothing else before hello.
                _ if peer.name.is_none() => {}
                ToServer::Hello { .. } => {}
                ToServer::Chat(text) => {
                    let Some(text) = clean_chat(&text).filter(|_| server.chat) else {
                        continue;
                    };
                    let peer = &mut *peer;
                    peer.chatted.retain(|&at| now - at < CHAT_WINDOW);
                    if peer.chatted.len() >= CHAT_LINES {
                        too_fast.insert(peer.id);
                        continue;
                    }
                    peer.chatted.push_back(now);
                    chat.push(ToClient::Chat {
                        from: peer.id,
                        text,
                    });
                }
                ToServer::Looks(looks) => {
                    peer.looks = Some(*looks);
                    peer.revision += 1;
                }
                ToServer::State(state) => {
                    peer.clock.heard(state.time, now);
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
    // In the chat: who joined (not told themselves) and who left.
    if server.chat {
        for (id, name) in &joined {
            let line = ToClient::Chat {
                from: SERVER,
                text: format!("{name} joined"),
            };
            for (_, mut peer, _) in &mut peers {
                if peer.name.is_some() && peer.id != *id {
                    peer.tell(&line);
                }
            }
        }
        for (_, name) in &left {
            chat.push(ToClient::Chat {
                from: SERVER,
                text: format!("{name} left"),
            });
        }
    }
    // Everyone there hears the chat, and then the ones held back why.
    for (_, mut peer, _) in &mut peers {
        if peer.name.is_none() || left.iter().any(|(id, _)| *id == peer.id) {
            continue;
        }
        for line in &chat {
            peer.tell(line);
        }
        if too_fast.contains(&peer.id) {
            peer.tell(&ToClient::Chat {
                from: SERVER,
                text: "Not so fast: wait a few seconds".to_string(),
            });
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
                peer.tell(&ToClient::Left { id });
            }
        }
    }
}

/// Sends each player the states of the cars near them (with their looks and name when
/// they're new to them or changed), each as often as its distance asks for, the traffic's
/// and the other players' alike; and every `1 / MAP_RATE` seconds the players on their
/// map.
fn send(
    mut server: ResMut<Server>,
    time: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    mut peers: Query<&mut Peer>,
    traffic: Query<(&TrafficCar, &Looks, &Motion)>,
) {
    let now = now(&time);
    if now - server.sent < 1.0 / SEND_RATE {
        return;
    }
    server.sent = now;
    let map_due = now - server.map_sent >= 1.0 / MAP_RATE - 0.5 / SEND_RATE;
    if map_due {
        server.map_sent = now;
    }
    let map = server.map;
    if now - server.pings_sent >= 1.0 / PING_RATE {
        server.pings_sent = now;
        let pings: Vec<(PlayerId, u16)> = (peers.iter())
            .filter(|peer| peer.name.is_some())
            .filter_map(|peer| {
                let ms = peer.link.rtt()?.as_secs_f64() * 1000.0;
                Some((peer.id, ms.round().min(u16::MAX.into()) as u16))
            })
            .collect();
        let message = ToClient::Pings(pings);
        for mut peer in &mut peers {
            if peer.name.is_some() {
                peer.tell(&message);
            }
        }
    }

    // The cars there are to see: the players' heard from lately, with looks, and the
    // traffic, all on our clock.
    struct Car {
        id: CarId,
        state: CarState,
        looks: Looks,
        revision: u32,
        tag: Option<String>,
        /// Whose it is, for the map.
        player: Option<PlayerId>,
    }
    let stamp = super::stamp(&time, &fixed);
    let players = peers
        .iter()
        .filter(|peer| now - peer.heard <= FROZEN)
        .filter_map(|peer| {
            let state = peer.state?;
            Some(Car {
                id: peer.car,
                state: CarState {
                    time: peer.clock.ours(state.time)?,
                    ..state
                },
                looks: peer.looks.clone()?,
                revision: peer.revision,
                tag: peer.name.clone().filter(|_| server.name_tags),
                player: Some(peer.id),
            })
        });
    let traffic = traffic.iter().map(|(car, looks, motion)| {
        let at = car.transform(looks);
        Car {
            id: car.id,
            state: CarState {
                time: stamp,
                position: at.translation,
                rotation: at.rotation,
                velocity: car.center_velocity(looks),
                angular_velocity: car.body.angular_velocity,
                motion: *motion,
            },
            looks: looks.clone(),
            revision: 1,
            tag: None,
            player: None,
        }
    });
    let cars: Vec<Car> = players.chain(traffic).collect();
    let cell = |p: Vec3| ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, car) in cars.iter().enumerate() {
        grid.entry(cell(car.state.position)).or_default().push(i);
    }
    let reach = (NEAR / CELL).ceil() as i32;
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
        let peer = &mut *peer;
        let (cx, cz) = cell(own.position);
        let mut states = Vec::new();
        let mut messages = Vec::new();
        let mut near = HashSet::new();
        for x in cx - reach..=cx + reach {
            for z in cz - reach..=cz + reach {
                for &i in grid.get(&(x, z)).into_iter().flatten() {
                    let car = &cars[i];
                    let distance = car.state.position.distance(own.position);
                    if car.id == peer.car || distance > NEAR {
                        continue;
                    }
                    near.insert(car.id);
                    let told = peer.told.entry(car.id).or_insert(Told {
                        revision: 0,
                        tag: None,
                        due: 0.0,
                    });
                    if told.revision != car.revision {
                        told.revision = car.revision;
                        messages.push(ToClient::Looks {
                            car: car.id,
                            looks: Box::new(car.looks.clone()),
                        });
                    }
                    if told.tag != car.tag {
                        told.tag.clone_from(&car.tag);
                        messages.push(ToClient::Tag {
                            car: car.id,
                            name: car.tag.clone(),
                        });
                    }
                    if now >= told.due {
                        states.push((car.id, car.state));
                        told.due = due_after(distance);
                    }
                }
            }
        }
        // What's out of range (or gone) goes, and is told afresh when it's back.
        peer.told.retain(|&car, _| {
            let kept = near.contains(&car);
            if !kept {
                messages.push(ToClient::Gone { car });
            }
            kept
        });
        for message in &messages {
            peer.tell(message);
        }
        for chunk in states.chunks(STATES_PER_MESSAGE) {
            peer.tell(&ToClient::Cars(chunk.to_vec()));
        }

        if map_due {
            let shown: Vec<MapPlayer> = (cars.iter())
                .filter_map(|car| Some((car, car.player?)))
                .filter(|&(_, player)| player != peer.id)
                .filter(|(car, _)| match map {
                    MapPlayers::Nobody => false,
                    MapPlayers::Near(within) => car.state.position.distance(own.position) <= within,
                    MapPlayers::Everyone => true,
                })
                .map(|(car, player)| MapPlayer {
                    id: player,
                    position: car.state.position.xz(),
                    velocity: car.state.velocity.xz(),
                    forward: (car.state.rotation * Vec3::NEG_Z)
                        .xz()
                        .normalize_or(Vec2::NEG_Y),
                    paint: car.looks.build.paint,
                })
                .collect();
            for chunk in shown.chunks(MAP_PER_MESSAGE) {
                peer.tell(&ToClient::Map(chunk.to_vec()));
            }
        }
    }
}
