//! What players and the server say to each other.
//!
//! A client says hello (protocol version, the map's fingerprint, the player's name), then
//! sends its car's looks whenever they change (reliably) and its state ~30 times a second
//! (unreliably: a lost one is soon replaced). The server welcomes it (or refuses it),
//! tells everyone who joins and leaves, and sends each player the states of the cars near
//! them, the nearer the more often, with their looks before the first state and after
//! every change, and the traffic near them the same way (which car, how it looks, where).

use bevy::prelude::*;

use super::wire::{Reader, Wire};
use crate::audio::{EngineCharacter, EngineRun, ExhaustSound};
use crate::chassis::{CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, SUSPENSIONS};
use crate::looks::{AxleLooks, Looks, Motion, WheelMotion};
use crate::physics::collision::Obb;
use crate::physics::engine::families::Layout;
use crate::physics::suspension::ArmGeometry;
use crate::physics::tire::COMPOUNDS;
use crate::wheels::{Rim, TireSize, Wheel};

/// Changes whenever the messages do: a client and server must agree on it.
pub const PROTOCOL: u32 = 2;

/// A player, numbered by the server.
pub type PlayerId = u32;

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
}

/// Server to client.
#[derive(Clone, Debug, PartialEq)]
pub enum ToClient {
    Welcome { you: PlayerId },
    Refused(String),
    Joined { id: PlayerId, name: String },
    Left { id: PlayerId },
    Looks { id: PlayerId, looks: Box<Looks> },
    States(Vec<(PlayerId, CarState)>),
    Traffic(Vec<TrafficState>),
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
        !matches!(self, ToClient::States(_) | ToClient::Traffic(_))
    }
}

/// Where a car is and how it moves, at `time` on its driver's clock (s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarState {
    pub time: f64,
    /// The chassis frame (the centre of mass) and its velocities.
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub motion: Motion,
}

/// A traffic car: which, what it looks like, and where it is at `time` on the server's
/// clock (s). Its reference point is on the ground midway between the axles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrafficState {
    pub id: u32,
    /// In `CHASSIS` and `PAINTS`.
    pub chassis: usize,
    pub paint: usize,
    pub time: f64,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    /// How far its wheels have rolled (m), and its front wheels' steering angle (rad).
    pub rolled: f32,
    pub steer: f32,
}

impl Wire for TrafficState {
    fn put(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.chassis.put(out);
        self.paint.put(out);
        self.time.put(out);
        self.position.put(out);
        self.rotation.put(out);
        self.velocity.put(out);
        self.angular_velocity.put(out);
        self.rolled.put(out);
        self.steer.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let state = Self {
            id: r.get()?,
            chassis: r.get()?,
            paint: r.get()?,
            time: r.get()?,
            position: r.get()?,
            rotation: r.get()?,
            velocity: r.get()?,
            angular_velocity: r.get()?,
            rolled: r.get()?,
            steer: r.get()?,
        };
        // A chassis or paint a newer server has and this game doesn't: shown as the first
        // one, rather than not at all.
        Some(Self {
            chassis: if state.chassis < CHASSIS.len() {
                state.chassis
            } else {
                0
            },
            paint: if state.paint < PAINTS.len() {
                state.paint
            } else {
                0
            },
            ..state
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
            _ => return None,
        })
    }
}

impl Wire for ToClient {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            ToClient::Welcome { you } => {
                0u8.put(out);
                you.put(out);
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
            ToClient::Looks { id, looks } => {
                Self::LOOKS.put(out);
                id.put(out);
                looks.put(out);
            }
            ToClient::States(states) => {
                5u8.put(out);
                states.put(out);
            }
            ToClient::Traffic(cars) => {
                6u8.put(out);
                cars.put(out);
            }
        }
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => ToClient::Welcome { you: r.get()? },
            1 => ToClient::Refused(r.get()?),
            2 => ToClient::Joined {
                id: r.get()?,
                name: r.get()?,
            },
            3 => ToClient::Left { id: r.get()? },
            Self::LOOKS => ToClient::Looks {
                id: r.get()?,
                looks: Box::new(r.get()?),
            },
            5 => ToClient::States(r.get()?),
            6 => ToClient::Traffic(r.get()?),
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
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            time: r.get()?,
            position: r.get()?,
            rotation: r.get()?,
            velocity: r.get()?,
            angular_velocity: r.get()?,
            motion: r.get()?,
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
                id: 3,
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
        };
        let bytes = encode(&ToServer::State(state));
        assert!(bytes.len() < 130, "{} bytes", bytes.len());
        assert_eq!(decode::<ToServer>(&bytes), Some(ToServer::State(state)));
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
