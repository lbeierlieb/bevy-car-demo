//! What can be seen and heard of a car, apart from its simulation: `Looks` (what it's
//! built like, which only changes when it's rebuilt) and `Motion` (what moves, every
//! frame). Visuals and sound only read these, so a car can be drawn and heard just as
//! well when it's simulated somewhere else (another player's car), with its entity's
//! `Transform` as the chassis frame (the centre of mass). A simulated `Car` fills its
//! own every frame.

use bevy::prelude::*;

use crate::audio::{EngineCharacter, EngineRun};
use crate::car::{Car, CarSetup, sync_transforms};
use crate::chassis::CarBuild;
use crate::physics::suspension::{ArmGeometry, CornerPose};

pub struct LooksPlugin;

impl Plugin for LooksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show_cars.after(sync_transforms));
    }
}

/// What a car is built like, as far as it's seen and heard.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Looks {
    /// The chassis, paint and wheels (the body is made from these).
    pub build: CarBuild,
    /// Where the body's design frame (on the ground midway between the axles) is in the
    /// chassis frame.
    pub body_offset: Vec3,
    /// Front and rear.
    pub axles: [AxleLooks; 2],
    pub engine: EngineCharacter,
}

/// An axle's linkage and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxleLooks {
    pub geometry: ArmGeometry,
    /// How far behind the centre of mass it is (m).
    pub z: f32,
    /// The tire's rolling radius (m).
    pub radius: f32,
}

impl Looks {
    pub fn of(setup: &CarSetup) -> Self {
        let axle = |front: bool| AxleLooks {
            geometry: setup.axle(front).geometry,
            z: if front { setup.front_z } else { setup.rear_z },
            radius: setup.axle(front).tire.radius,
        };
        Self {
            build: setup.build.clone(),
            body_offset: setup.body_offset,
            axles: [axle(true), axle(false)],
            engine: EngineCharacter::of(&setup.engine),
        }
    }

    /// The pose of `corner` (front left, front right, rear left, rear right) as `wheel`
    /// says, in the chassis frame.
    pub fn corner_pose(&self, corner: usize, wheel: &WheelMotion) -> CornerPose {
        let axle = &self.axles[corner / 2];
        let side = if corner.is_multiple_of(2) { -1.0 } else { 1.0 };
        axle.geometry.pose(wheel.theta, wheel.steer, side, axle.z)
    }
}

/// What moves on a car.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    /// Front left, front right, rear left, rear right.
    pub wheels: [WheelMotion; 4],
    pub engine: EngineRun,
}

/// A wheel's suspension, steering and spin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelMotion {
    /// The lower arm's angle (see `ArmGeometry`).
    pub theta: f32,
    /// The steering angle (rad, positive turns left).
    pub steer: f32,
    /// How far it has turned (rad, positive rolls forward).
    pub spin: f32,
}

impl Motion {
    pub fn of(car: &Car) -> Self {
        Self {
            wheels: car.corners.each_ref().map(|corner| WheelMotion {
                theta: corner.theta,
                steer: corner.steer,
                spin: corner.spin,
            }),
            engine: EngineRun::of(&car.drivetrain),
        }
    }
}

/// Every simulated car shows what it's like and how it moves.
pub fn show_cars(
    mut commands: Commands,
    mut cars: Query<(Entity, &Car, Option<&mut Looks>, Option<&mut Motion>)>,
) {
    for (entity, car, looks, motion) in &mut cars {
        match (looks, motion) {
            (Some(mut looks), Some(mut motion)) => {
                looks.set_if_neq(Looks::of(&car.setup));
                *motion = Motion::of(car);
            }
            _ => {
                commands
                    .entity(entity)
                    .insert((Looks::of(&car.setup), Motion::of(car)));
            }
        }
    }
}
