//! What players and the server say to each other.
//!
//! A client says hello (protocol version, the map's fingerprint, the player's name), then
//! sends its car's looks whenever they change (reliably) and its state ~30 times a second
//! (unreliably: a lost one is soon replaced). The server welcomes it (or refuses it) and
//! tells it where the shops are (its own list, separate from the map), then
//! tells everyone who joins and leaves, and sends each player the states of the cars near
//! them, the nearer the more often, with their looks before the first state and after
//! every change, the name a car shows, if any, and when a car is out of range or gone. Every car is sent alike, the traffic's
//! and the other players' (numbered by one counter, never by the player), so nothing on
//! the wire tells them apart but the names the server chooses to show.
//! About once a second it also sends where the other players are for the map, as many of
//! them as the server's settings show, and every two seconds everyone's ping. If the
//! server has chat on (it says so in its welcome), players' chat messages go to everyone,
//! with the server's own lines (who joined and left) among them.

use bevy::prelude::*;

use super::wire::{Reader, Wire};
use crate::audio::{EngineCharacter, EngineRun, ExhaustSound};
use crate::chassis::{CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, SUSPENSIONS};
use crate::looks::{AxleLooks, Looks, Motion, WheelMotion};
use crate::physics::collision::Obb;
use crate::physics::engine::families::Layout;
use crate::physics::suspension::ArmGeometry;
use crate::physics::tire::COMPOUNDS;
use crate::shops::Shop;
use crate::wheels::{Rim, TireSize, Wheel};

/// Changes whenever the messages do: a client and server must agree on it.
pub const PROTOCOL: u32 = 6;

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
    Hello {
        protocol: u32,
        map: u64,
        name: String,
    },
    Looks(Box<Looks>),
    State(CarState),
    /// A chat message for everyone.
    Chat(String),
}

/// Server to client.
#[derive(Clone, Debug, PartialEq)]
pub enum ToClient {
    Welcome {
        you: PlayerId,
        /// Whether the server has chat on.
        chat: bool,
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
}

impl ToServer {
    /// The first byte of a `Looks` message.
    pub const LOOKS: u8 = 1;

    /// Whether it must arrive (else a newer one will do).
    pub fn reliable(&self) -> bool {
        !matches!(self, ToServer::State(_))
    }
}

impl ToClient {
    /// The first byte of a `Looks` message.
    pub const LOOKS: u8 = 4;

    pub fn reliable(&self) -> bool {
        !matches!(
            self,
            ToClient::Cars(_) | ToClient::Map(_) | ToClient::Pings(_)
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
}

/// Another player's car on the map: where it is and goes (xz, on the ground), which way
/// it points and its paint (in `PAINTS`).
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
        // A paint a newer server has: the first one.
        Some(Self {
            paint: if player.paint < PAINTS.len() {
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
            } => {
                0u8.put(out);
                protocol.put(out);
                map.put(out);
                name.put(out);
            }
            ToServer::Looks(looks) => {
                Self::LOOKS.put(out);
                looks.put(out);
            }
            ToServer::State(state) => {
                2u8.put(out);
                state.put(out);
            }
            ToServer::Chat(text) => {
                3u8.put(out);
                text.put(out);
            }
        }
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => ToServer::Hello {
                protocol: r.get()?,
                map: r.get()?,
                name: r.get()?,
            },
            Self::LOOKS => ToServer::Looks(Box::new(r.get()?)),
            2 => ToServer::State(r.get()?),
            3 => ToServer::Chat(r.get()?),
            _ => return None,
        })
    }
}

impl Wire for ToClient {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            ToClient::Welcome { you, chat } => {
                0u8.put(out);
                you.put(out);
                chat.put(out);
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
        }
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => ToClient::Welcome {
                you: r.get()?,
                chat: r.get()?,
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
        })
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
        self.load.put(out);
        self.turbo_speed.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            running: r.get()?,
            rpm: r.get()?,
            load: r.get()?,
            turbo_speed: r.get()?,
        })
    }
}

/// Looks are checked when read: catalogue indices in range and sizes that make sense,
/// since they're turned into meshes and sound.
impl Wire for Looks {
    fn put(&self, out: &mut Vec<u8>) {
        self.build.put(out);
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
            && sane(looks.engine.turbo_limit, 0.0, 1e5);
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
        self.limited_slip.put(out);
        self.suspension.put(out);
        self.bars.put(out);
        self.wheels.put(out);
        self.camber.put(out);
        self.pressure.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let build = Self {
            chassis: r.get()?,
            paint: r.get()?,
            gearbox: r.get()?,
            clutch: r.get()?,
            final_drive: r.get()?,
            limited_slip: r.get()?,
            suspension: r.get()?,
            bars: r.get()?,
            wheels: r.get()?,
            camber: r.get()?,
            pressure: r.get()?,
        };
        let chassis = CHASSIS.get(build.chassis)?;
        let fine = build.paint < PAINTS.len()
            && build.gearbox < GEARBOXES.len()
            && build.clutch < CLUTCHES.len()
            && build.suspension < SUSPENSIONS.len()
            && (0..2).all(|i| build.bars[i] < chassis.bars[i].sizes.len())
            && build.wheels.iter().all(|w| {
                w.compound < COMPOUNDS.len()
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
        self.turbo_limit.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let layout = *Layout::ALL.get(r.get::<usize>()?)?;
        let exhaust = ExhaustSound {
            muffle: r.get::<f32>()?.clamp(0.0, 1.0),
            bore: r.get::<f32>()?.clamp(0.3, 3.0),
            header: r.get::<f32>()?.clamp(20.0, 2000.0),
            turbine: r.get::<f32>()?.clamp(0.0, 1.0),
        };
        Some(Self {
            layout,
            exhaust,
            turbo_limit: r.get()?,
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
    use crate::net::wire::{decode, encode};

    #[test]
    fn every_factory_car_looks_the_same_after_the_trip() {
        for chassis in 0..CHASSIS.len() {
            let looks = Looks::of(&CarSetup::factory(chassis, 0));
            let message = ToServer::Looks(Box::new(looks.clone()));
            let bytes = encode(&message);
            assert_eq!(decode::<ToServer>(&bytes), Some(message), "{chassis}");
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
        };
        let bytes = encode(&ToServer::State(state));
        assert!(bytes.len() < 130, "{} bytes", bytes.len());
        assert_eq!(decode::<ToServer>(&bytes), Some(ToServer::State(state)));
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
        let shops = crate::shops::parse(crate::shops::DEFAULT).unwrap();
        let message = ToClient::Shops(shops);
        assert_eq!(decode::<ToClient>(&encode(&message)), Some(message));
    }

    #[test]
    fn looks_out_of_the_catalogues_are_refused() {
        let mut looks = Looks::of(&CarSetup::default());
        looks.build.chassis = CHASSIS.len();
        assert_eq!(
            decode::<ToServer>(&encode(&ToServer::Looks(Box::new(looks)))),
            None
        );
    }
}
