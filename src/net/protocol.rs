//! What players and the server say to each other.
//!
//! A client says hello (protocol version, the map's fingerprint, the player's name), then
//! sends its car's looks whenever they change (reliably) and its state ~30 times a second
//! (unreliably: a lost one is soon replaced). For the server to drive the same car, it
//! also sends what the car is built of whenever that changes, the driver's input in every
//! simulation step (unreliably, each with the few before it) and now and then the car's
//! whole state after a step. The server welcomes it (or refuses it) and
//! tells it where the shops and events are (its own lists, separate from the map), then
//! tells everyone who joins and leaves, and sends each player the states of the cars near
//! them, the nearer the more often, with their looks before the first state and after
//! every change, the name a car shows, if any, and when a car is out of range or gone. Every car is sent alike, the traffic's
//! and the other players' (numbered by one counter, never by the player), so nothing on
//! the wire tells them apart but the names the server chooses to show.
//! About once a second it also sends where the other players are for the map, as many of
//! them as the server's settings show, and every two seconds everyone's ping. If the
//! server has chat on (it says so in its welcome), players' chat messages go to everyone,
//! with the server's own lines (who joined and left) among them.

use std::sync::Arc;

use bevy::prelude::*;

use super::wire::{Blob, Reader, Wire};
use crate::audio::{EngineCharacter, EngineRun, ExhaustSound};
use crate::car::DriverInput;
use crate::catalogue::Shared;
use crate::chassis::{CarBuild, FACTORY_TOE};
use crate::events::Event;
use crate::events::round::{Action, Round};
use crate::fitting::Work;
use crate::heli::HeliPose;
use crate::looks::{AxleLooks, Looks, Motion, WheelMotion};
use crate::owned::{OwnedView, Stash};
use crate::physics::collision::Obb;
use crate::physics::engine::families::Layout;
use crate::physics::suspension::ArmGeometry;
use crate::shops::Shop;
use crate::spec::CarSpec;
use crate::terrain::Spawn;
use crate::trade::{Market, Stock, Trade};
use crate::wheels::{Rim, TireSize, Wheel};

/// Changes whenever the messages do: a client and server must agree on it.
pub const PROTOCOL: u32 = 71;

/// A player, numbered by the server.
pub type PlayerId = u32;

/// A car the server sends: a traffic car or a player's, numbered by the server (not by
/// the player: which car is whose isn't said).
pub type CarId = u32;

/// The `from` of the server's own chat lines (players are numbered from 1).
pub const SERVER: PlayerId = 0;
/// The longest chat message (characters).
pub const CHAT_LENGTH: usize = 200;

/// A chat message as it's passed on: one line (control characters such as line breaks
/// made spaces), trimmed, at most `CHAT_LENGTH` characters; nothing if that leaves nothing.
pub fn clean_chat(text: &str) -> Option<String> {
    let line: String = (text.chars())
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line: String = line.trim().chars().take(CHAT_LENGTH).collect();
    let line = line.trim_end().to_string();
    (!line.is_empty()).then_some(line)
}

/// Client to server.
#[derive(Clone, Debug, PartialEq)]
pub enum ToServer {
    /// Joining: the game's protocol and map, and who's joining (an account's name and
    /// password; a guest's name without one).
    Hello {
        protocol: u32,
        map: u64,
        name: String,
        password: String,
    },
    State(CarState),
    /// A chat message for everyone.
    Chat(String),
    /// About the events: into a lobby, out, the hunted's settings and choices.
    Event(Action),
    /// Request `id`: works done to our car at the shop it's at (see `fitting`), all or
    /// none; answered with the car (`YourCar`), what we own and why it wasn't done, if it
    /// wasn't (`Owned`), then `Done`.
    Work {
        id: u32,
        works: Vec<Work>,
    },
    /// A car from the car dealer we're at: the catalogue's trim `trim`; the car we drove
    /// goes into the garage, or is traded in (gone, with what's in its trunk).
    Buy {
        trim: u16,
        trade_in: bool,
    },
    /// The maps of the engine whose hardware has `fingerprint` (`maps::fingerprint`), if
    /// the server has them (a factory engine's).
    Maps {
        fingerprint: u64,
    },
    /// The driver's input in the simulation steps up to `last`, the latest last (the
    /// last few each time, so a lost message loses nothing).
    Inputs {
        last: u32,
        inputs: Vec<DriverInput>,
    },
    /// Our car's whole state (`snapshot`) after simulation step `step`, now and then (for
    /// the server's log of how well it follows).
    Check {
        step: u32,
        state: Vec<u8>,
    },
    /// Our car was put somewhere (or changed) outside the simulation after step `step`:
    /// its whole state then.
    Place {
        step: u32,
        state: Vec<u8>,
    },
    /// Something done with what we own: a part loaded or unloaded, shipped, another car
    /// driven (see `owned`).
    Stash(Stash),
    /// The special rescue: our car (one that can't drive) carried to the shop `to` (its
    /// place in the list: a gas station or a parts shop).
    Rescue {
        to: u16,
    },
    /// Something asked of the shop that trades we're at (see `trade`): answered with what
    /// it has (`Stock`) and what we own (`Owned`).
    Trade(Trade),
}

/// Server to client.
#[derive(Clone, Debug, PartialEq)]
pub enum ToClient {
    Welcome {
        you: PlayerId,
        /// Our car's number (the server sends our own car's state like any other while it
        /// has it, see `Yours::notice`).
        car: CarId,
        /// Whether the server has chat on.
        chat: bool,
        /// Where players start.
        spawn: Spawn,
    },
    Refused(String),
    Joined {
        id: PlayerId,
        name: String,
    },
    Left {
        id: PlayerId,
    },
    Looks {
        car: CarId,
        looks: Box<Looks>,
    },
    /// Where cars are and how they move.
    Cars(Vec<(CarId, CarState)>),
    /// The name a car shows over it from now on, or none.
    Tag {
        car: CarId,
        name: Option<String>,
    },
    /// A car out of range, or gone altogether.
    Gone {
        car: CarId,
    },
    /// Other players to show on the map (a player missing from every message for a while
    /// is no longer shown).
    Map(Vec<MapPlayer>),
    /// Every player's round trip to the server (ms), ours too.
    Pings(Vec<(PlayerId, u16)>),
    /// A chat line from a player (ours too, back from the server) or the server (`SERVER`).
    Chat {
        from: PlayerId,
        text: String,
    },
    /// The shops on the map, told after the welcome.
    Shops(Vec<Shop>),
    /// The events on the map, told after the shops.
    Events(Vec<Event>),
    /// The round we're in (its lobby or under way), whenever it changes; none once out.
    Round(Option<Round>),
    /// Who's in which event's round (its place in the list), everyone; whenever it changes.
    Busy(Vec<(PlayerId, u16)>),
    /// Where the helicopter over the hunted is, to everyone in a car hunt's running
    /// round, wherever they are, a few times a second.
    Heli(HeliPose),
    /// How many simulation steps before the server needed them our inputs came lately (at
    /// the least; negative: too late), a few times a second.
    Pace {
        early: i16,
    },
    /// Our car's whole state as the server drives it, ten times a second, and at once
    /// when the server moved it.
    Yours(Yours),
    /// What cars and parts there are, told after the welcome: every car is built from it.
    Catalogue(Shared),
    /// Our car is now built as this (on joining, lent for a round or given back, bought);
    /// its state comes in `Yours`.
    YourCar(Box<CarSpec>),
    /// The engine maps asked for (`maps::EngineMaps::to_text`, zlib-compressed), or none
    /// if the server hasn't got them.
    Maps {
        fingerprint: u64,
        text: Option<Blob>,
    },
    /// What we own besides the car we drive, after joining and whenever it changes; and
    /// why something we asked (`ToServer::Stash`, `Work`) wasn't done, if it wasn't.
    Owned {
        owned: Box<OwnedView>,
        why: Option<String>,
    },
    /// Our request `id` (`ToServer::Work`) is answered: the `YourCar` before it is
    /// the car as it is now.
    Done {
        id: u32,
    },
    /// What the shop that trades we're at has, after we asked (`ToServer::Trade`).
    Stock(Box<Stock>),
    /// The offers at the marketplace, after we asked or changed something there.
    Market(Box<Market>),
}

/// A player's car's whole state as the server drives it: after their step `step`, which
/// ended at `time` on the server's clock (the clock every car's state is stamped on: the
/// player's game shows the other cars as of each of its steps by it), with the latest of their placements the server had taken up then and how many times it had
/// moved the car itself; and, while the server has the car (the rescue helicopter carrying
/// it), what to tell the player about that: their game then doesn't drive it, it shows it
/// as the server sends it to everyone (`Cars`), until it's given back as a move.
#[derive(Clone, Debug, PartialEq)]
pub struct Yours {
    pub step: u32,
    pub time: f64,
    pub placed: Option<u32>,
    pub moved: u32,
    pub notice: Option<String>,
    pub state: Vec<u8>,
}

impl ToServer {
    /// Whether it must arrive (else a newer one will do).
    pub fn reliable(&self) -> bool {
        !matches!(self, ToServer::State(_) | ToServer::Inputs { .. })
    }
}

impl ToClient {
    /// The first byte of a `Looks` message.
    pub const LOOKS: u8 = 4;
    /// The first byte of a `Catalogue` message.
    pub const CATALOGUE: u8 = 18;

    pub fn reliable(&self) -> bool {
        !matches!(
            self,
            ToClient::Cars(_)
                | ToClient::Map(_)
                | ToClient::Pings(_)
                | ToClient::Heli(_)
                | ToClient::Pace { .. }
                | ToClient::Yours(_)
        )
    }
}

/// Where a car is and how it moves, at `time` on its sender's clock (s): the driver's
/// to the server, the server's to the players.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarState {
    pub time: f64,
    /// The chassis frame (the centre of mass) and its velocities.
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub motion: Motion,
    /// A ghost: stopped at a shop (or just left one), nothing collides with it.
    pub ghost: bool,
    /// Nobody drives it, the server moves it (its rescue helicopter has it): nothing a
    /// game does changes where it goes, so games show it as it went, a little late
    /// (between states), rather than guessing ahead.
    pub scripted: bool,
    /// The rescue helicopter that's coming for the car, carrying it or flying off (its
    /// time on the same clock as the state's).
    pub heli: Option<HeliPose>,
}

/// Another player's car on the map: where it is and goes (xz, on the ground), which way
/// it points and its paint (in the catalogue's `paints`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPlayer {
    pub id: PlayerId,
    pub position: Vec2,
    pub velocity: Vec2,
    /// A unit vector.
    pub forward: Vec2,
    pub paint: usize,
}

impl Wire for MapPlayer {
    fn put(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.position.put(out);
        self.velocity.put(out);
        self.forward.put(out);
        self.paint.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let player = Self {
            id: r.get()?,
            position: r.get()?,
            velocity: r.get()?,
            forward: r.get::<Vec2>()?.normalize_or(Vec2::NEG_Y),
            paint: r.get()?,
        };
        // A paint the catalogue doesn't have: the first one.
        let paints = r.catalogue().map_or(0, |catalogue| catalogue.paints.len());
        Some(Self {
            paint: if player.paint < paints {
                player.paint
            } else {
                0
            },
            ..player
        })
    }
}

impl Wire for ToServer {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            ToServer::Hello {
                protocol,
                map,
                name,
                password,
            } => {
                0u8.put(out);
                protocol.put(out);
                map.put(out);
                name.put(out);
                password.put(out);
            }
            ToServer::State(state) => {
                2u8.put(out);
                state.put(out);
            }
            ToServer::Chat(text) => {
                3u8.put(out);
                text.put(out);
            }
            ToServer::Event(action) => {
                4u8.put(out);
                action.put(out);
            }
            ToServer::Work { id, works } => {
                13u8.put(out);
                id.put(out);
                works.put(out);
            }
            ToServer::Inputs { last, inputs } => {
                6u8.put(out);
                last.put(out);
                inputs.put(out);
            }
            ToServer::Check { step, state } => {
                7u8.put(out);
                step.put(out);
                state.put(out);
            }
            ToServer::Place { step, state } => {
                8u8.put(out);
                step.put(out);
                state.put(out);
            }
            ToServer::Buy { trim, trade_in } => {
                9u8.put(out);
                trim.put(out);
                trade_in.put(out);
            }
            ToServer::Maps { fingerprint } => {
                10u8.put(out);
                fingerprint.put(out);
            }
            ToServer::Stash(stash) => {
                11u8.put(out);
                stash.put(out);
            }
            ToServer::Rescue { to } => {
                12u8.put(out);
                to.put(out);
            }
            ToServer::Trade(trade) => {
                14u8.put(out);
                trade.put(out);
            }
        }
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => ToServer::Hello {
                protocol: r.get()?,
                map: r.get()?,
                name: r.get()?,
                // Older games say no password: they're told the version is wrong.
                password: if r.done() { String::new() } else { r.get()? },
            },
            2 => ToServer::State(r.get()?),
            3 => ToServer::Chat(r.get()?),
            4 => ToServer::Event(r.get()?),
            13 => ToServer::Work {
                id: r.get()?,
                works: crate::fitting::read_works(r)?,
            },
            6 => ToServer::Inputs {
                last: r.get()?,
                inputs: r.get()?,
            },
            7 => ToServer::Check {
                step: r.get()?,
                state: r.get()?,
            },
            8 => ToServer::Place {
                step: r.get()?,
                state: r.get()?,
            },
            9 => ToServer::Buy {
                trim: r.get()?,
                trade_in: r.get()?,
            },
            10 => ToServer::Maps {
                fingerprint: r.get()?,
            },
            11 => ToServer::Stash(r.get()?),
            12 => ToServer::Rescue { to: r.get()? },
            14 => ToServer::Trade(r.get()?),
            _ => return None,
        })
    }
}

impl Wire for ToClient {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            ToClient::Welcome {
                you,
                car,
                chat,
                spawn,
            } => {
                0u8.put(out);
                you.put(out);
                car.put(out);
                chat.put(out);
                spawn.put(out);
            }
            ToClient::Refused(why) => {
                1u8.put(out);
                why.put(out);
            }
            ToClient::Joined { id, name } => {
                2u8.put(out);
                id.put(out);
                name.put(out);
            }
            ToClient::Left { id } => {
                3u8.put(out);
                id.put(out);
            }
            ToClient::Looks { car, looks } => {
                Self::LOOKS.put(out);
                car.put(out);
                looks.put(out);
            }
            ToClient::Cars(cars) => {
                5u8.put(out);
                cars.put(out);
            }
            ToClient::Tag { car, name } => {
                6u8.put(out);
                car.put(out);
                name.put(out);
            }
            ToClient::Map(players) => {
                7u8.put(out);
                players.put(out);
            }
            ToClient::Pings(pings) => {
                8u8.put(out);
                pings.put(out);
            }
            ToClient::Chat { from, text } => {
                9u8.put(out);
                from.put(out);
                text.put(out);
            }
            ToClient::Gone { car } => {
                10u8.put(out);
                car.put(out);
            }
            ToClient::Shops(shops) => {
                11u8.put(out);
                shops.put(out);
            }
            ToClient::Events(events) => {
                12u8.put(out);
                events.put(out);
            }
            ToClient::Round(round) => {
                13u8.put(out);
                round.put(out);
            }
            ToClient::Busy(busy) => {
                14u8.put(out);
                busy.put(out);
            }
            ToClient::Heli(pose) => {
                15u8.put(out);
                pose.put(out);
            }
            ToClient::Pace { early } => {
                16u8.put(out);
                early.put(out);
            }
            ToClient::Yours(yours) => {
                17u8.put(out);
                yours.step.put(out);
                yours.time.put(out);
                yours.placed.put(out);
                yours.moved.put(out);
                yours.notice.put(out);
                yours.state.put(out);
            }
            ToClient::Catalogue(catalogue) => {
                Self::CATALOGUE.put(out);
                catalogue.put(out);
            }
            ToClient::YourCar(spec) => {
                19u8.put(out);
                spec.put(out);
            }
            ToClient::Maps { fingerprint, text } => {
                20u8.put(out);
                fingerprint.put(out);
                text.put(out);
            }
            ToClient::Owned { owned, why } => {
                21u8.put(out);
                owned.put(out);
                why.put(out);
            }
            ToClient::Done { id } => {
                22u8.put(out);
                id.put(out);
            }
            ToClient::Stock(stock) => {
                23u8.put(out);
                stock.put(out);
            }
            ToClient::Market(market) => {
                24u8.put(out);
                market.put(out);
            }
        }
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => ToClient::Welcome {
                you: r.get()?,
                car: r.get()?,
                chat: r.get()?,
                spawn: r.get()?,
            },
            1 => ToClient::Refused(r.get()?),
            2 => ToClient::Joined {
                id: r.get()?,
                name: r.get()?,
            },
            3 => ToClient::Left { id: r.get()? },
            Self::LOOKS => ToClient::Looks {
                car: r.get()?,
                looks: Box::new(r.get()?),
            },
            5 => ToClient::Cars(r.get()?),
            6 => ToClient::Tag {
                car: r.get()?,
                name: r.get()?,
            },
            7 => ToClient::Map(r.get()?),
            8 => ToClient::Pings(r.get()?),
            9 => ToClient::Chat {
                from: r.get()?,
                text: r.get()?,
            },
            10 => ToClient::Gone { car: r.get()? },
            11 => ToClient::Shops(r.get()?),
            12 => ToClient::Events(r.get()?),
            13 => ToClient::Round(r.get()?),
            14 => ToClient::Busy(r.get()?),
            15 => ToClient::Heli(r.get()?),
            16 => ToClient::Pace { early: r.get()? },
            17 => ToClient::Yours(Yours {
                step: r.get()?,
                time: r.get()?,
                placed: r.get()?,
                moved: r.get()?,
                notice: r.get()?,
                state: r.get()?,
            }),
            Self::CATALOGUE => ToClient::Catalogue(Arc::new(r.get()?)),
            19 => ToClient::YourCar(Box::new(r.get()?)),
            20 => ToClient::Maps {
                fingerprint: r.get()?,
                text: r.get()?,
            },
            21 => ToClient::Owned {
                owned: Box::new(r.get()?),
                why: r.get()?,
            },
            22 => ToClient::Done { id: r.get()? },
            23 => ToClient::Stock(Box::new(r.get()?)),
            24 => ToClient::Market(Box::new(r.get()?)),
            _ => return None,
        })
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
        self.1.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some((r.get()?, r.get()?))
    }
}

impl Wire for CarState {
    fn put(&self, out: &mut Vec<u8>) {
        self.time.put(out);
        self.position.put(out);
        self.rotation.put(out);
        self.velocity.put(out);
        self.angular_velocity.put(out);
        self.motion.put(out);
        self.ghost.put(out);
        self.scripted.put(out);
        self.heli.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            time: r.get()?,
            position: r.get()?,
            rotation: r.get()?,
            velocity: r.get()?,
            angular_velocity: r.get()?,
            motion: r.get()?,
            ghost: r.get()?,
            scripted: r.get()?,
            heli: r.get()?,
        })
    }
}

impl Wire for HeliPose {
    fn put(&self, out: &mut Vec<u8>) {
        self.time.put(out);
        self.position.put(out);
        self.velocity.put(out);
        self.rotation.put(out);
        self.cable.put(out);
        self.hooked.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let pose = Self {
            time: r.get()?,
            position: r.get()?,
            velocity: r.get()?,
            rotation: r.get()?,
            cable: r.get()?,
            hooked: r.get()?,
        };
        (pose.cable >= 0.0 && pose.cable < 100.0).then_some(pose)
    }
}

impl Wire for Motion {
    fn put(&self, out: &mut Vec<u8>) {
        self.wheels.put(out);
        self.engine.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            wheels: r.get()?,
            engine: r.get()?,
        })
    }
}

impl Wire for WheelMotion {
    fn put(&self, out: &mut Vec<u8>) {
        self.theta.put(out);
        self.steer.put(out);
        self.spin.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            theta: r.get()?,
            steer: r.get()?,
            spin: r.get()?,
        })
    }
}

impl Wire for EngineRun {
    fn put(&self, out: &mut Vec<u8>) {
        self.running.put(out);
        self.rpm.put(out);
        self.fill.put(out);
        self.turbo_speed.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            running: r.get()?,
            rpm: r.get()?,
            fill: r.get::<f32>()?.clamp(0.0, 4.0),
            turbo_speed: r.get()?,
        })
    }
}

/// Looks are checked when read: catalogue indices in range and sizes that make sense,
/// since they're turned into meshes and sound.
impl Wire for Looks {
    fn put(&self, out: &mut Vec<u8>) {
        self.build.put(out);
        self.car.put(out);
        self.body_offset.put(out);
        self.axles.put(out);
        self.engine.put(out);
        self.hull.put(out);
        self.mass.put(out);
        self.inertia.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let looks = Self {
            build: r.get()?,
            car: r.get()?,
            body_offset: r.get()?,
            axles: r.get()?,
            engine: r.get()?,
            hull: r.get()?,
            mass: r.get()?,
            inertia: r.get()?,
        };
        let sane = |x: f32, low: f32, high: f32| (low..=high).contains(&x);
        let fine = sane(looks.body_offset.length(), 0.0, 5.0)
            && looks.axles.iter().all(|a| {
                sane(a.z.abs(), 0.0, 5.0)
                    && sane(a.radius, 0.1, 1.0)
                    && [
                        a.geometry.lower_pivot,
                        a.geometry.lower_ball,
                        a.geometry.upper_pivot,
                        a.geometry.upper_ball,
                        a.geometry.wheel_center,
                        a.geometry.spring_top,
                    ]
                    .iter()
                    .all(|p| sane(p.length(), 0.0, 3.0))
            })
            && sane(looks.hull.half.max_element(), 0.1, 10.0)
            && sane(looks.mass, 100.0, 10_000.0)
            && sane(looks.inertia.min_element(), 1.0, 100_000.0)
            && sane(looks.engine.turbo_limit, 0.0, 1e5)
            && sane(looks.engine.displacement, 1e-4, 0.02);
        fine.then_some(looks)
    }
}

impl Wire for CarBuild {
    fn put(&self, out: &mut Vec<u8>) {
        self.chassis.put(out);
        self.paint.put(out);
        self.gearbox.put(out);
        self.clutch.put(out);
        self.final_drive.put(out);
        self.suspension.put(out);
        self.bars.put(out);
        self.wheels.put(out);
        self.camber.put(out);
        self.pressure.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let catalogue = r.catalogue()?;
        let build = Self {
            catalogue: catalogue.clone(),
            chassis: r.get()?,
            paint: r.get()?,
            gearbox: r.get()?,
            clutch: r.get()?,
            final_drive: r.get()?,
            suspension: r.get()?,
            bars: r.get()?,
            wheels: r.get()?,
            camber: r.get()?,
            pressure: r.get()?,
            // Not sent: nobody else sees them.
            toe: FACTORY_TOE,
            brake_force: 1.0,
            brake_balance: 0.0,
            ride_height: [0.0; 2],
            wing: 0.0,
        };
        let chassis = catalogue.chassis.get(build.chassis)?;
        let fine = build.paint < catalogue.paints.len()
            && build.gearbox < catalogue.gearboxes.len()
            && build.clutch < catalogue.clutches.len()
            && build.suspension < catalogue.suspensions.len()
            && (0..2).all(|i| build.bars[i] < chassis.bars[i].sizes.len())
            && build.wheels.iter().all(|w| {
                w.compound < catalogue.compounds.len()
                    && (10..=24).contains(&w.rim.diameter)
                    && (100..=1000).contains(&w.tire.width)
                    && (20..=90).contains(&w.tire.aspect)
            });
        fine.then_some(build)
    }
}

impl Wire for Wheel {
    fn put(&self, out: &mut Vec<u8>) {
        self.rim.diameter.put(out);
        self.rim.width.put(out);
        self.tire.width.put(out);
        self.tire.aspect.put(out);
        self.tire.rim.put(out);
        self.compound.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            rim: Rim {
                diameter: r.get()?,
                width: r.get()?,
            },
            tire: TireSize {
                width: r.get()?,
                aspect: r.get()?,
                rim: r.get()?,
            },
            compound: r.get()?,
        })
    }
}

impl Wire for AxleLooks {
    fn put(&self, out: &mut Vec<u8>) {
        self.geometry.put(out);
        self.z.put(out);
        self.radius.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            geometry: r.get()?,
            z: r.get()?,
            radius: r.get()?,
        })
    }
}

impl Wire for ArmGeometry {
    fn put(&self, out: &mut Vec<u8>) {
        self.lower_pivot.put(out);
        self.lower_ball.put(out);
        self.upper_pivot.put(out);
        self.upper_ball.put(out);
        self.wheel_center.put(out);
        self.spring_top.put(out);
        self.spring_arm_fraction.put(out);
        self.static_camber.put(out);
        self.static_toe.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            lower_pivot: r.get()?,
            lower_ball: r.get()?,
            upper_pivot: r.get()?,
            upper_ball: r.get()?,
            wheel_center: r.get()?,
            spring_top: r.get()?,
            spring_arm_fraction: r.get()?,
            static_camber: r.get()?,
            static_toe: r.get()?,
        })
    }
}

impl Wire for EngineCharacter {
    fn put(&self, out: &mut Vec<u8>) {
        let layout = Layout::ALL.iter().position(|&l| l == self.layout);
        layout.unwrap_or(0).put(out);
        self.exhaust.muffle.put(out);
        self.exhaust.bore.put(out);
        self.exhaust.header.put(out);
        self.exhaust.turbine.put(out);
        (self.exhaust.turbos as u8).put(out);
        self.turbo_limit.put(out);
        self.displacement.put(out);
        self.whine.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let layout = *Layout::ALL.get(r.get::<usize>()?)?;
        let exhaust = ExhaustSound {
            muffle: r.get::<f32>()?.clamp(0.0, 1.0),
            bore: r.get::<f32>()?.clamp(0.3, 3.0),
            header: r.get::<f32>()?.clamp(20.0, 2000.0),
            turbine: r.get::<f32>()?.clamp(0.0, 1.0),
            turbos: f32::from(r.get::<u8>()?.min(12)),
        };
        Some(Self {
            layout,
            exhaust,
            turbo_limit: r.get()?,
            displacement: r.get()?,
            whine: r.get::<f32>()?.clamp(0.0, 100.0),
        })
    }
}

impl Wire for Obb {
    fn put(&self, out: &mut Vec<u8>) {
        self.center.put(out);
        self.axes.put(out);
        self.half.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            center: r.get()?,
            axes: r.get()?,
            half: r.get()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;
    use crate::net::wire::encode;

    /// Reads a message with cars of the built-in catalogue.
    fn decode<T: Wire>(bytes: &[u8]) -> Option<T> {
        crate::net::wire::decode_with(bytes, Some(&crate::catalogue::builtin()))
    }

    #[test]
    fn every_factory_car_looks_the_same_after_the_trip() {
        for chassis in 0..crate::catalogue::builtin().chassis.len() {
            let looks = Looks::of(&CarSetup::factory(chassis, 0));
            let reply = ToClient::Looks {
                car: 3,
                looks: Box::new(looks),
            };
            assert_eq!(decode::<ToClient>(&encode(&reply)), Some(reply));
        }
    }

    #[test]
    fn a_state_is_small() {
        let state = CarState {
            time: 12.5,
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::from_rotation_y(0.3),
            velocity: Vec3::X,
            angular_velocity: Vec3::Y,
            motion: Motion::default(),
            ghost: true,
            scripted: false,
            heli: None,
        };
        let bytes = encode(&ToServer::State(state));
        assert!(bytes.len() < 130, "{} bytes", bytes.len());
        assert_eq!(decode::<ToServer>(&bytes), Some(ToServer::State(state)));
        // With a helicopter coming for it.
        let heli = HeliPose {
            time: 12.4,
            position: Vec3::new(4.0, 60.0, -3.0),
            velocity: Vec3::X * 20.0,
            rotation: Quat::from_rotation_y(1.0),
            cable: 11.5,
            hooked: true,
        };
        let state = CarState {
            heli: Some(heli),
            ..state
        };
        let bytes = encode(&ToServer::State(state));
        assert!(bytes.len() < 180, "{} bytes", bytes.len());
        assert_eq!(decode::<ToServer>(&bytes), Some(ToServer::State(state)));
        let message = ToClient::Heli(heli);
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
    }

    #[test]
    fn tags_come_through() {
        for name in [Some("Anna".to_string()), None] {
            let message = ToClient::Tag { car: 7, name };
            assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
        }
    }

    #[test]
    fn the_shops_come_through() {
        let shops = crate::shops::parse(crate::shops::LIST).unwrap();
        let message = ToClient::Shops(shops);
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
    }

    #[test]
    fn rounds_come_through() {
        use crate::events::hunt::{Cars, Catch, Ending, Outcome as HuntOutcome, Settings};
        use crate::events::round::*;
        let settings = Settings {
            cars: Cars::Factory,
            hunters_reset: false,
            ..Settings::default()
        };
        let mut round = Round {
            event: 0,
            players: vec![3, 1, 2],
            host: 1,
            game: Game::Hunt(settings),
            phase: Phase::Lobby {
                last: Some(Outcome::Hunt(HuntOutcome {
                    hunted: "Anna".into(),
                    ending: Ending::Caught(253.0),
                })),
            },
            best: Vec::new(),
        };
        let message = ToClient::Round(Some(round.clone()));
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
        round.game = Game::Hunt(Settings {
            catch: Catch::Stop,
            ..settings
        });
        round.phase = Phase::Lobby {
            last: Some(Outcome::Hunt(HuntOutcome {
                hunted: "Anna".into(),
                ending: Ending::Tagged {
                    by: "Ben".into(),
                    after: 61.0,
                },
            })),
        };
        let message = ToClient::Round(Some(round.clone()));
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
        round.phase = Phase::Running {
            start: 12.5,
            play: Play::Hunt { slow: None },
        };
        let message = ToClient::Round(Some(round));
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
        for action in [
            Action::Enter(2),
            Action::Leave,
            Action::Game(Game::Hunt(Settings::default())),
            Action::Host(4),
            Action::Start,
            Action::Touched(17),
        ] {
            let message = ToServer::Event(action);
            assert_eq!(decode::<ToServer>(&encode(&message)), Some(message));
        }
    }

    #[test]
    fn the_events_come_through() {
        let events = crate::events::parse(crate::events::LIST).unwrap();
        let message = ToClient::Events(events);
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
    }

    #[test]
    fn looks_out_of_the_catalogues_are_refused() {
        let mut looks = Looks::of(&CarSetup::default());
        looks.build.chassis = crate::catalogue::builtin().chassis.len();
        let message = ToClient::Looks {
            car: 3,
            looks: Box::new(looks),
        };
        assert_eq!(decode::<ToClient>(&encode(&message)), None);
    }
}
