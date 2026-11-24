//! Car setups, spawning and driver input.

use bevy::prelude::*;

use crate::chassis::{CLUTCHES, CarBuild, Chassis, DEFAULT_CHASSIS, SUSPENSIONS};
use crate::heli::rescue::Rescue;
use crate::input::WheelInput;
use crate::net::wire::{Reader, Wire, decode, encode};
use crate::physics::collision::Obb;
use crate::physics::drivetrain::{Differential, Drive, Drivetrain, EngineSetup, REVERSE_SPEED};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::parts::Parts;
use crate::physics::fuel::{self, Fuel, Tank};
use crate::physics::rigid_body::RigidBody;
use crate::physics::suspension::{ArmGeometry, AxleSetup, Corner};
use crate::physics::tire::TireParams;

use crate::storage::Folder;
use crate::terrain::{Spawn, Terrain};
use crate::wheels::{HUB_INERTIA, Wheel};

/// Everything the simulation needs to know about a car, worked out from what it's built
/// of: a chassis and what's fitted to it (`build`), and an engine.
#[derive(Clone, Debug)]
pub struct CarSetup {
    pub name: &'static str,
    pub color: Color,
    /// What the car is built of, apart from the engine (see `engine.parts`).
    pub build: CarBuild,
    /// Sprung mass without fuel (kg), and its principal moments of inertia (kg·m²).
    pub sprung_mass: f32,
    pub inertia: Vec3,
    /// The fuel tank, and where its centre is in the chassis frame.
    pub tank: Tank,
    pub tank_at: Vec3,
    /// Axle positions along the chassis z axis (forward is -z), from the centre of mass.
    pub front_z: f32,
    pub rear_z: f32,
    pub front: AxleSetup,
    pub rear: AxleSetup,
    pub max_steer: f32,
    /// Engine, clutch, gearbox and differentials.
    pub engine: EngineSetup,
    /// Total brake torque, the front axle's share of it, and handbrake torque per rear wheel.
    pub brake_torque: f32,
    pub brake_bias: f32,
    pub handbrake_torque: f32,
    /// Drag coefficient times frontal area (m²).
    pub drag_area: f32,
    /// Downforce coefficient times area (m²) over the front and the rear axle.
    pub downforce_area: [f32; 2],
    /// Where the body's design frame (its origin on the ground, midway between the
    /// axles) is in the chassis frame, whose origin is the centre of mass.
    pub body_offset: Vec3,
    /// The driver's eyes in the chassis frame.
    pub eye: Vec3,
    /// The box around the body (and the wheels, where they stand out), in the chassis
    /// frame: what hits the ground, houses, trees and other cars.
    pub hull: Obb,
}

impl CarSetup {
    /// The car that `build` and an engine of `parts` (built as `engine`) make. The
    /// engine's and the gearbox's masses sit where the chassis has them, which sets the
    /// balance; springs and dampers are matched to the weight on each axle, and the
    /// brakes' balance to how it shifts forward when braking.
    pub fn new(build: CarBuild, parts: Parts, engine: EngineBuild) -> Self {
        let chassis = build.chassis();
        let gearbox = build.gearbox();
        let body = &chassis.body;
        let engine_mass = parts.family().built_mass(&engine.hardware);
        // Masses and where they are behind the front axle.
        let masses = [
            (chassis.mass, chassis.mass_at),
            (engine_mass, chassis.bay.engine_at),
            (gearbox.mass, chassis.bay.gearbox_at),
        ];
        let sprung_mass: f32 = masses.iter().map(|(mass, _)| mass).sum();
        let centre = masses.iter().map(|(mass, at)| mass * at).sum::<f32>() / sprung_mass;
        let (front_z, rear_z) = (-centre, chassis.wheelbase - centre);
        // The body as a box, and each mass's distance from the common centre.
        let size = Vec3::new(0.9 * body.width, 0.7, 0.95 * body.length);
        let spread: f32 = masses
            .iter()
            .map(|(mass, at)| mass * (at - centre).powi(2))
            .sum();
        let inertia = RigidBody::box_inertia(chassis.mass, size) + Vec3::new(spread, spread, 0.0);

        let tank = chassis.tank;
        let tank_at = Vec3::new(0.0, tank.height - chassis.cg_height, front_z + tank.at);
        // Springs, tires and brakes are matched to the car with half a tank.
        let fuel_mass = 0.5 * tank.litres * fuel::DENSITY;
        let laden = sprung_mass + fuel_mass;
        let front_share =
            (sprung_mass * rear_z + fuel_mass * (rear_z - tank_at.z)) / (laden * chassis.wheelbase);
        let (_, ride, damping, ..) = SUSPENSIONS[build.suspension];
        let share = |i: usize| {
            if i == 0 {
                front_share
            } else {
                1.0 - front_share
            }
        };
        let unsprung = |i: usize| chassis.hub[i] + build.wheels[i].mass();
        // Each tire's load at rest.
        let static_load = |i: usize| (laden * share(i) / 2.0 + unsprung(i)) * 9.81;
        // The tires' friction at rest (warm and new), averaged over the car by load: of the
        // ones fitted, and of the factory's, which the brakes are made for.
        let grip_of = |wheels: &[Wheel; 2]| {
            let load = |i: usize| static_load(i);
            let friction = |i: usize| wheels[i].tire.params(wheels[i].compound).friction(load(i));
            (friction(0) * load(0) + friction(1) * load(1)) / (load(0) + load(1))
        };
        let (grip, factory_grip) = (grip_of(&build.wheels), grip_of(&chassis.wheels));
        let axle = |i: usize| {
            let wheel = &build.wheels[i];
            let radius = wheel.tire.radius();
            let share = share(i);
            let mut geometry = ArmGeometry::default();
            // The stock linkage, moved out to the track and to the wheel's height below
            // the centre of mass.
            let shift = Vec2::new(chassis.track[i] / 2.0, radius - chassis.cg_height)
                - geometry.wheel_center;
            for point in [
                &mut geometry.lower_pivot,
                &mut geometry.lower_ball,
                &mut geometry.upper_pivot,
                &mut geometry.upper_ball,
                &mut geometry.wheel_center,
                &mut geometry.spring_top,
            ] {
                *point += shift;
            }
            geometry.static_camber = build.camber[i].to_radians();
            // Toe-in at the rear keeps the car stable, especially off throttle (the factory's).
            geometry.static_toe = build.toe[i].to_radians();
            let unsprung_mass = unsprung(i);
            let mut axle = AxleSetup {
                geometry,
                spring_rate: 0.0,
                damping: 0.0,
                // Rolling the body by φ moves one wheel up by track × φ against the other.
                roll_bar: chassis.bars[i].stiffness(build.bars[i]).to_degrees()
                    / chassis.track[i].powi(2),
                unsprung_mass,
                wheel_inertia: wheel.inertia() + HUB_INERTIA * chassis.hub[i],
                tire: TireParams {
                    inflation: build.pressure[i],
                    ..wheel.tire.params(wheel.compound)
                },
            };
            axle.tune(
                chassis.ride[i] * ride,
                chassis.damping * damping,
                laden * share / 2.0,
            );
            axle
        };

        let mass = laden + 2.0 * (unsprung(0) + unsprung(1));
        let radius = (build.wheels[0].tire.radius() + build.wheels[1].tire.radius()) / 2.0;
        // Braking as hard as the tires can, the weight shifts forward by the height of
        // the centre of mass over the wheelbase; a little more to the front keeps the
        // rear wheels from locking first.
        // Rear-wheel drive adds the engine's braking at the back.
        let margin = if chassis.drive == Drive::Rear {
            0.06
        } else {
            0.03
        };
        let brake_bias = front_share + grip * chassis.cg_height / chassis.wheelbase + margin;
        let differential = if build.limited_slip {
            Differential::LIMITED_SLIP
        } else {
            Differential::OPEN
        };
        let body_offset = Vec3::new(0.0, -chassis.cg_height, front_z + chassis.wheelbase / 2.0);
        let [eye_x, eye_y, eye_left] = chassis.eye;
        let wheels = chassis.wheels();
        let mut hull = body_box(chassis, &build.wheels);
        hull.center += body_offset;
        Self {
            name: chassis.name,
            color: build.paint(),
            sprung_mass,
            inertia,
            tank,
            tank_at,
            front_z,
            rear_z,
            front: axle(0),
            rear: axle(1),
            max_steer: chassis.max_steer,
            engine: EngineSetup {
                parts,
                build: engine,
                clutch_torque: gearbox.clutch_torque * CLUTCHES[build.clutch].1,
                gears: gearbox.gears,
                reverse: gearbox.reverse,
                final_drive: build.final_drive,
                shift_time: gearbox.shift_time,
                engage_time: gearbox.engage_time,
                drive: chassis.drive,
                differential,
                centre: differential,
            },
            brake_torque: chassis.brakes * factory_grip * mass * 9.81 * radius * build.brake_force,
            brake_bias: (brake_bias.min(0.9) + build.brake_balance).clamp(0.3, 0.95),
            handbrake_torque: 2.0 * mass,
            drag_area: chassis.drag_area + tire_drag(&build.wheels, &chassis.wheels),
            downforce_area: chassis.downforce_area,
            body_offset,
            eye: body_offset + Vec3::new(-eye_left, eye_y, body.z(eye_x, &wheels)),
            hull,
            build,
        }
    }

    /// A chassis (in `CHASSIS`) as it leaves the factory, with the factory's engine
    /// `option` for it.
    pub fn factory(chassis: usize, option: usize) -> Self {
        let build = CarBuild::factory(chassis);
        let parts = build.factory_engine(option);
        let engine = EngineBuild::new(parts.hardware(), parts.ecu);
        Self::new(build, parts, engine)
    }

    pub fn axle(&self, front: bool) -> &AxleSetup {
        if front { &self.front } else { &self.rear }
    }

    /// Sprung mass carried by each wheel of the front or the rear axle (kg) with `litres`
    /// of fuel.
    fn corner_mass_with(&self, front: bool, litres: f32) -> f32 {
        let wheelbase = self.rear_z - self.front_z;
        let fuel = litres * fuel::DENSITY;
        let (body, tank) = if front {
            (self.rear_z, self.rear_z - self.tank_at.z)
        } else {
            (-self.front_z, self.tank_at.z - self.front_z)
        };
        (self.sprung_mass * body + fuel * tank) / wheelbase / 2.0
    }

    /// Sprung mass carried by each wheel of the front or the rear axle with half a tank,
    /// which the springs are matched to (kg).
    pub fn corner_mass(&self, front: bool) -> f32 {
        self.corner_mass_with(front, self.tank.litres / 2.0)
    }

    /// Mass of the whole car with `litres` of fuel (kg).
    pub fn mass_with(&self, litres: f32) -> f32 {
        let unsprung = 2.0 * (self.front.unsprung_mass + self.rear.unsprung_mass);
        self.sprung_mass + litres * fuel::DENSITY + unsprung
    }

    /// Share of the car's weight on the front axle with `litres` of fuel.
    pub fn front_weight_with(&self, litres: f32) -> f32 {
        let unsprung = self.front.unsprung_mass;
        (2.0 * self.corner_mass_with(true, litres) + 2.0 * unsprung) / self.mass_with(litres)
    }
}

/// Drag area per m² of a tire's front (width times diameter): about half of it stands in
/// the air, at a drag coefficient of about 0.6.
const TIRE_DRAG: f32 = 0.3;

/// Drag area (m²) `wheels` add over the factory's (less for smaller tires): the chassis'
/// own drag area is with those.
fn tire_drag(wheels: &[Wheel; 2], factory: &[Wheel; 2]) -> f32 {
    let front = |wheel: &Wheel| wheel.tire.width_m() * 2.0 * wheel.tire.radius();
    (0..2)
        .map(|i| 2.0 * TIRE_DRAG * (front(&wheels[i]) - front(&factory[i])))
        .sum()
}

/// The box around a chassis' body (and the `fitted` wheels, where they stand out), in the
/// body's design frame: its origin on the ground midway between the axles, forward -z.
pub fn body_box(chassis: &Chassis, fitted: &[Wheel; 2]) -> Obb {
    let body = &chassis.body;
    let wheels = chassis.wheels();
    let top = (body.deck.iter().map(|p| p[1]))
        .chain(body.cabin.iter().map(|p| p[1]))
        .fold(0.0, f32::max);
    let (nose, tail) = (body.z(0.0, &wheels), body.z(body.length, &wheels));
    let half_width = (0..2)
        .map(|i| (chassis.track[i] + fitted[i].tire.width_m()) / 2.0)
        .fold(body.width / 2.0, f32::max);
    Obb {
        center: Vec3::new(0.0, (body.floor + top) / 2.0, (nose + tail) / 2.0),
        axes: [Vec3::X, Vec3::Y, Vec3::Z],
        half: Vec3::new(half_width, (top - body.floor) / 2.0, (tail - nose) / 2.0),
    }
}

impl Default for CarSetup {
    /// The car the game starts with, with its base engine.
    fn default() -> Self {
        Self::factory(DEFAULT_CHASSIS, 0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// 0 to 1 (full throttle).
    pub throttle: f32,
    pub brake: f32,
    /// Clutch pedal travel (0 released, 1 pressed), if there's a pedal; without one the
    /// clutch works by itself.
    pub clutch: Option<f32>,
    /// -1 (right) to 1 (left), as a fraction of full lock.
    pub steer: f32,
    /// 0 to 1.
    pub handbrake: f32,
}

#[derive(Component, Clone)]
pub struct Car {
    /// Which car it is (see `ActiveCar`).
    pub index: usize,
    pub setup: CarSetup,
    pub body: RigidBody,
    pub corners: [Corner; 4],
    pub drivetrain: Drivetrain,
    pub controls: Controls,
    /// What the front wheels do to the steering (positive turns them left), for force
    /// feedback.
    pub rack: crate::sim::steering::Rack,
    /// Traction control: whether it's on, and the share of the driver's throttle it
    /// lets through right now.
    pub traction_control: bool,
    pub traction: f32,
    /// ABS: whether it's on, and the share of the brake pedal it lets through to each
    /// wheel right now.
    pub abs: bool,
    pub abs_pressure: [f32; 4],
    /// The fuel in the tank.
    pub fuel: Fuel,
    /// How hard the body (not the tires) has hit houses, trees, other cars and the ground
    /// since this was last taken (the crash sounds take it every frame): the speed the
    /// knocks changed (m/s), added up.
    pub knocks: f32,
    /// The parking brake (see `parking_brake_rule`): Space (or a handbrake button) pulls it
    /// on at a standstill or releases it, gas releases it. Cars start with it on.
    pub parking_brake: bool,
    /// The ignition button switched the engine off and is still held (it doesn't crank
    /// until let go).
    pub switching_off: bool,
}

impl Car {
    /// A car at rest, slightly above the ground at (x, z), facing along `yaw`.
    pub fn spawn_at(
        index: usize,
        setup: CarSetup,
        terrain: &Terrain,
        x: f32,
        z: f32,
        yaw: f32,
    ) -> Self {
        let height = setup.build.chassis().cg_height + 0.15;
        // On top of whatever is there (a bridge's deck).
        let position = Vec3::new(x, terrain.ground(x, z, f32::INFINITY).height + height, z);
        Self::new(index, setup, position, Quat::from_rotation_y(yaw))
    }

    fn new(index: usize, setup: CarSetup, position: Vec3, rotation: Quat) -> Self {
        let body = RigidBody::new(position, rotation, setup.sprung_mass, setup.inertia);
        let fuel = Fuel::new(setup.tank.litres);
        let corners =
            [(-1.0, true), (1.0, true), (-1.0, false), (1.0, false)].map(|(side, front)| {
                let z = if front { setup.front_z } else { setup.rear_z };
                Corner::new(setup.axle(front), side, z, front)
            });
        Self {
            index,
            drivetrain: Drivetrain::new(&setup.engine),
            setup,
            body,
            corners,
            controls: Controls::default(),
            rack: Default::default(),
            traction_control: true,
            traction: 1.0,
            abs: true,
            abs_pressure: [1.0; 4],
            fuel,
            knocks: 0.0,
            parking_brake: true,
            switching_off: false,
        }
        .with_fuel_load()
    }

    /// The body's mass, balance and inertia with the fuel that's in the tank now.
    pub fn load_fuel(&mut self) {
        let body = &mut self.body;
        body.mass = self.setup.sprung_mass;
        body.inertia = self.setup.inertia;
        body.center = Vec3::ZERO;
        body.load(self.fuel.mass(), self.setup.tank_at);
    }

    fn with_fuel_load(mut self) -> Self {
        self.load_fuel();
        self
    }

    /// Fills (or drains) the tank to `fill` (a share of it).
    pub fn refuel(&mut self, fill: f32) {
        self.fuel = Fuel::new(fill.clamp(0.0, 1.0) * self.setup.tank.litres);
        self.load_fuel();
    }

    /// Where on the road nearest to it the car goes back to (xz, and the yaw it faces
    /// there): in its lane, facing the way along it nearest to where the car faced; where
    /// it is if there's no road.
    pub fn road_spot(&self, terrain: &Terrain) -> (Vec2, f32) {
        let p = self.body.position.xz();
        let yaw = heading(self.body.rotation);
        let facing = Vec2::new(-yaw.sin(), -yaw.cos());
        terrain.drive_spot(p, facing).unwrap_or((p, yaw))
    }

    /// Puts the car at rest at (x, z) in the xz plane, facing along `yaw`, as it was: its
    /// setup, fuel, tires and engine wear, and the driver's settings. In neutral with the
    /// parking brake on (in gear, a clutch pedal let up would stall it at once).
    pub fn place(&mut self, terrain: &Terrain, p: Vec2, yaw: f32) {
        let mut placed = Self::spawn_at(self.index, self.setup.clone(), terrain, p.x, p.y, yaw);
        placed.drivetrain.neutral();
        placed.parking_brake = true;
        placed.keep_settings(self);
        placed.fuel = self.fuel;
        placed.load_fuel();
        placed.drivetrain.engine.damage = self.drivetrain.engine.damage;
        placed.drivetrain.engine.thermal = self.drivetrain.engine.thermal;
        for (corner, old) in placed.corners.iter_mut().zip(&self.corners) {
            corner.tread = old.tread;
        }
        *self = placed;
    }

    /// The car rebuilt as `setup` where it is and as it moves; its engine keeps its
    /// state and wear if it's still the same engine.
    pub fn rebuilt(&self, setup: CarSetup) -> Self {
        let same_engine = setup.engine.parts == self.setup.engine.parts;
        let mut car = Self::new(self.index, setup, self.body.position, self.body.rotation);
        car.body.velocity = self.body.velocity;
        car.body.angular_velocity = self.body.angular_velocity;
        car.controls = self.controls;
        car.parking_brake = self.parking_brake;
        car.keep_settings(self);
        if same_engine {
            car.drivetrain.engine = self.drivetrain.engine;
        }
        let (new, old) = (&car.setup.build, &self.setup.build);
        // Each axle keeps its tires' state if they're the same tires.
        let same_tires = |front: bool| {
            let axle = usize::from(!front);
            new.chassis == old.chassis && new.wheels[axle] == old.wheels[axle]
        };
        let same_tires = [true, false].map(same_tires);
        for (corner, old) in car.corners.iter_mut().zip(&self.corners) {
            corner.omega = old.omega;
            if same_tires[usize::from(!corner.front)] {
                corner.tread = old.tread;
            }
        }
        car.fuel = self.fuel;
        car.fuel.fit(&car.setup.tank);
        car.load_fuel();
        car
    }

    /// Takes the driver's settings (traction control, ABS) from the car this one replaces.
    /// (The active car gets them from `DriverAids` anyway.)
    pub fn keep_settings(&mut self, old: &Car) {
        self.traction_control = old.traction_control;
        self.abs = old.abs;
    }

    /// Brings every tread to its compound's best temperature, as after a few laps.
    #[cfg(test)]
    pub fn warm_tires(&mut self) {
        for corner in &mut self.corners {
            corner
                .tread
                .warm(self.setup.axle(corner.front).tire.compound.best);
        }
    }

    pub fn forward_speed(&self) -> f32 {
        self.body.velocity.dot(self.body.rotation * Vec3::NEG_Z)
    }

    /// What the speedometer reads: the gearbox output's speed (m/s), the driven wheels'
    /// rims averaged the way the differentials do. Reads high with wheelspin, zero with
    /// the wheels locked or the car carried.
    pub fn shaft_speed(&self) -> f32 {
        let shares = self.setup.engine.drive.shares();
        (self.corners.iter().zip(shares))
            .map(|(corner, share)| share * corner.omega * self.setup.axle(corner.front).tire.radius)
            .sum()
    }
}

crate::snapshot::snapshot!(Controls {
    throttle,
    brake,
    clutch,
    steer,
    handbrake
});

/// The car's state as it drives (see `snapshot`); its setup stays, and the body's mass
/// follows the fuel loaded.
impl crate::snapshot::Snapshot for Car {
    fn save(&self, out: &mut Vec<u8>) {
        self.body.save(out);
        self.corners.save(out);
        self.drivetrain.save(out);
        self.controls.save(out);
        self.rack.save(out);
        self.traction_control.save(out);
        self.traction.save(out);
        self.abs.save(out);
        self.abs_pressure.save(out);
        self.fuel.save(out);
        self.knocks.save(out);
        self.parking_brake.save(out);
        self.switching_off.save(out);
    }

    fn load(&mut self, r: &mut crate::net::wire::Reader) -> Option<()> {
        crate::snapshot::Snapshot::load(&mut self.body, r)?;
        self.corners.load(r)?;
        self.drivetrain.load(r)?;
        self.controls.load(r)?;
        self.rack.load(r)?;
        self.traction_control.load(r)?;
        self.traction.load(r)?;
        self.abs.load(r)?;
        self.abs_pressure.load(r)?;
        self.fuel.load(r)?;
        self.knocks.load(r)?;
        self.parking_brake.load(r)?;
        self.switching_off.load(r)?;
        // What another build's state could hold that this one hasn't got.
        if !self.drivetrain.fits(&self.setup.engine) {
            return None;
        }
        self.fuel.fit(&self.setup.tank);
        self.load_fuel();
        Some(())
    }
}

/// Yaw angle of an orientation's forward (-z) direction.
pub fn heading(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::NEG_Z;
    (-forward.x).atan2(-forward.z)
}

/// The car currently driven (and followed by the camera).
#[derive(Resource, Default)]
pub struct ActiveCar(pub usize);

/// Settings kept between runs (`~/.config/car-sim/settings/*.cfg`).
pub(crate) const SETTINGS: Folder = Folder {
    cache: false,
    name: "settings",
    extension: "cfg",
    web_prefix: "car-sim-settings:",
};

/// The driver's choice of aids (traction control, ABS), the same for every car and kept between runs (`~/.config/car-sim/settings/aids.cfg`): `drive` hands it
/// to the active car.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct DriverAids {
    pub traction_control: bool,
    pub abs: bool,
}

impl Default for DriverAids {
    fn default() -> Self {
        Self {
            traction_control: true,
            abs: true,
        }
    }
}

impl DriverAids {
    /// The saved choice, or the defaults.
    pub fn load() -> Self {
        SETTINGS
            .load("aids")
            .map_or_else(Self::default, |text| Self::from_text(&text))
    }

    pub fn save(&self) {
        if let Err(err) = SETTINGS.save("aids", &self.to_text()) {
            warn!("Couldn't save the driving aids: {err}");
        }
    }

    fn to_text(self) -> String {
        format!(
            "traction_control {}\nabs {}\n",
            self.traction_control, self.abs
        )
    }

    /// Reads what `to_text` wrote; unknown or broken lines are skipped.
    fn from_text(text: &str) -> Self {
        let mut aids = Self::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(Ok(on))) = (words.next(), words.next().map(str::parse)) else {
                continue;
            };
            match key {
                "traction_control" => aids.traction_control = on,
                "abs" => aids.abs = on,
                _ => {}
            }
        }
        aids
    }
}

/// Driving the active car, and its entity following the simulation. Which cars there are
/// is up to the app.
pub struct CarPlugin;

impl Plugin for CarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveCar>()
            .insert_resource(DriverAids::load())
            .add_message::<ReverseRefused>()
            .init_resource::<Away>()
            .init_resource::<Held>()
            .init_resource::<Rescued>()
            .init_resource::<ResetRule>()
            .init_resource::<Presses>()
            .init_resource::<StepInput>()
            .add_systems(PreUpdate, gather_presses.after(crate::input::ReadWheel))
            .add_systems(
                FixedUpdate,
                (take_input, drive).chain().before(crate::sim::simulate),
            )
            .add_systems(Update, sync_transforms);
    }
}

/// Starts in the coupé with its base engine, on the road at the map's spawn point.
pub struct DefaultCarPlugin;

impl Plugin for DefaultCarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_default_car);
    }
}

/// R (or the wheel's reset button) calls the rescue helicopter, which carries the active
/// car back to the nearest road and sets it down on its wheels, as it was (fuel, tires and
/// engine wear stay); what an event's rules say otherwise (`ResetRule`).
pub struct ResetPlugin;

impl Plugin for ResetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, reset_car.before(sync_transforms));
    }
}

fn spawn_default_car(mut commands: Commands, terrain: Res<Terrain>, spawn: Res<Spawn>) {
    // On the road at the spawn point.
    let (p, yaw) = terrain.road_spawns(&spawn, 1, 0.0)[0];
    let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, p.x, p.y, yaw);
    // In neutral with the parking brake on, as joining a server or R leaves it.
    car.drivetrain.neutral();
    commands.spawn((Transform::from_translation(car.body.position), car));
}

/// How fast the keys turn the front wheels towards full lock, in full locks per second.
const KEY_STEER_RATE: f32 = 1.0;
/// How fast they come back towards the middle (released, or steering the other way).
const KEY_STEER_RETURN_RATE: f32 = 2.5;

/// The driver asked for reverse while the car was rolling (`drive` refused it).
#[derive(Message)]
pub struct ReverseRefused;
/// The driver is away from the wheel (in the dyno's room): the active car holds still,
/// as any other car does, and R does nothing.
#[derive(Resource, Default)]
pub struct Away(pub bool);

/// The active car is held at the start (a hunter during a car hunt's head start): its
/// brakes are on whatever the driver does (the engine still revs).
#[derive(Resource, Default)]
pub struct Held(pub bool);

/// The rescue helicopter is on its way to the active car or carrying it
/// (`heli::rescue`): the driver's input does nothing (the brakes are on), and online the
/// car is a ghost.
#[derive(Resource, Default)]
pub struct Rescued(pub bool);

/// What R does with the active car (in an event's round, its rules).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub enum ResetRule {
    /// Has the rescue helicopter carry it to the nearest road.
    #[default]
    Road,
    /// Has the rescue helicopter carry it to this spot (map x, z), facing along this yaw
    /// (a race's last gate passed).
    Spot(Vec2, f32),
    /// Nothing.
    Never,
}

/// What the driver does in one simulation step: the wheel's and pedals' positions, and
/// the buttons pressed since the step before (see `DriverInput::take`). A car's own rules
/// turn it into controls (`Car::drive`), the same way wherever it's simulated, so a
/// step's input is all that has to go to the server for it to drive the car alike.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriverInput {
    /// Steering by an axis, -1 (right) to 1 (left) of full lock.
    pub steer: Option<f32>,
    /// Steering by keys or buttons: -1 (right), 0 or 1 (left), to turn towards.
    pub steer_keys: f32,
    pub throttle: f32,
    pub brake: f32,
    /// Clutch pedal travel, if there's a pedal.
    pub clutch: Option<f32>,
    /// A handbrake lever's travel, if it's a lever.
    pub handbrake: Option<f32>,
    /// A handbrake button pressed since the last step, and held.
    pub handbrake_toggle: bool,
    pub handbrake_held: bool,
    /// With an H-shifter: the gate the lever is in (0 between gates).
    pub shifter: Option<i8>,
    /// Shifts up less shifts down pressed since the last step.
    pub shift: i8,
    /// The ignition button pressed since the last step, and held.
    pub ignition: bool,
    pub ignition_held: bool,
    /// Nobody's at the wheel (away in a shop's room, or the car carried by the rescue
    /// helicopter): all brakes, nothing else.
    pub away: bool,
    /// Held at the start: the brakes are on whatever the driver does.
    pub held: bool,
}

impl DriverInput {
    /// Takes this step's input: the wheel as it is now, and the presses gathered since the
    /// last step (which are then cleared, so each counts once however many steps a frame
    /// runs, and none is lost when a frame runs none). As precise as it goes over the
    /// network: the car is driven by exactly what the server is sent.
    pub fn take(wheel: &WheelInput, presses: &mut Presses) -> Self {
        let input = Self {
            steer: wheel.steer,
            steer_keys: wheel.steer_keys,
            throttle: wheel.throttle.unwrap_or(0.0),
            brake: wheel.brake.unwrap_or(0.0),
            clutch: wheel.clutch,
            handbrake: wheel.handbrake,
            handbrake_toggle: presses.handbrake,
            handbrake_held: wheel.handbrake_held,
            shifter: wheel.shifter,
            shift: presses.shift,
            ignition: presses.ignition,
            ignition_held: wheel.ignition_held,
            away: false,
            held: false,
        };
        *presses = Presses::default();
        input.sent()
    }

    /// The input as the other end reads it.
    pub fn sent(&self) -> Self {
        decode(&encode(self)).expect("an input reads back")
    }
}

/// A share of the way (0 to 1) in 16 bits.
fn put_share(x: f32, out: &mut Vec<u8>) {
    ((x.clamp(0.0, 1.0) * 65535.0).round() as u16).put(out);
}

fn get_share(r: &mut Reader) -> Option<f32> {
    Some(f32::from(r.get::<u16>()?) / 65535.0)
}

/// About ten bytes: which of the optional ones there are and the buttons as bits, the
/// axes in 16 bits (steering signed), the shifter's gate and the shifts.
impl Wire for DriverInput {
    fn put(&self, out: &mut Vec<u8>) {
        let bits = [
            self.steer.is_some(),
            self.clutch.is_some(),
            self.handbrake.is_some(),
            self.shifter.is_some(),
            self.handbrake_toggle,
            self.handbrake_held,
            self.ignition,
            self.ignition_held,
            self.away,
            self.held,
            self.steer_keys > 0.0,
            self.steer_keys < 0.0,
        ];
        let flags = (bits.iter().enumerate()).fold(0u16, |f, (i, &b)| f | (u16::from(b) << i));
        flags.put(out);
        if let Some(steer) = self.steer {
            ((steer.clamp(-1.0, 1.0) * 32767.0).round() as i16).put(out);
        }
        put_share(self.throttle, out);
        put_share(self.brake, out);
        if let Some(clutch) = self.clutch {
            put_share(clutch, out);
        }
        if let Some(handbrake) = self.handbrake {
            put_share(handbrake, out);
        }
        if let Some(gate) = self.shifter {
            gate.put(out);
        }
        self.shift.put(out);
    }

    fn get(r: &mut Reader) -> Option<Self> {
        let flags: u16 = r.get()?;
        let bit = |i: u32| flags & (1 << i) != 0;
        let steer = bit(0)
            .then(|| Some(f32::from(r.get::<i16>()?) / 32767.0))
            .flatten();
        if bit(0) && steer.is_none() {
            return None;
        }
        let throttle = get_share(r)?;
        let brake = get_share(r)?;
        let mut optional = |on: bool| -> Option<Option<f32>> {
            if on {
                get_share(r).map(Some)
            } else {
                Some(None)
            }
        };
        let clutch = optional(bit(1))?;
        let handbrake = optional(bit(2))?;
        let shifter = if bit(3) {
            let gate: i8 = r.get()?;
            (-1..=8).contains(&gate).then_some(gate)?;
            Some(gate)
        } else {
            None
        };
        Some(Self {
            steer: steer.map(|s| s.clamp(-1.0, 1.0)),
            steer_keys: f32::from(i8::from(bit(10)) - i8::from(bit(11))),
            throttle,
            brake,
            clutch,
            handbrake,
            handbrake_toggle: bit(4),
            handbrake_held: bit(5),
            shifter,
            shift: r.get()?,
            ignition: bit(6),
            ignition_held: bit(7),
            away: bit(8),
            held: bit(9),
        })
    }
}

/// The buttons pressed since the last simulation step (frames and steps don't line up).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Presses {
    pub shift: i8,
    pub ignition: bool,
    pub handbrake: bool,
}

/// The simulation step under way (or the last), numbered from the start, and the driver's
/// input to the active car in it.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct StepInput {
    pub step: u32,
    pub input: DriverInput,
}

/// Gathers the presses of this frame for the next simulation step.
fn gather_presses(wheel: Res<WheelInput>, mut presses: ResMut<Presses>) {
    presses.shift = presses
        .shift
        .saturating_add(wheel.shift_up as i8 - wheel.shift_down as i8);
    presses.ignition |= wheel.ignition;
    presses.handbrake |= wheel.handbrake_toggle;
}

/// This step's input from the wheel, and whether the driver is at it.
pub fn take_input(
    wheel: Res<WheelInput>,
    mut presses: ResMut<Presses>,
    (away, held, rescued): (Res<Away>, Res<Held>, Res<Rescued>),
    mut input: ResMut<StepInput>,
) {
    let mut taken = DriverInput::take(&wheel, &mut presses);
    taken.away = away.0 || rescued.0;
    taken.held = held.0;
    input.step = input.step.wrapping_add(1);
    input.input = taken;
}

/// The step's input drives the active car, with the driver's aids; any other car holds
/// all brakes.
pub fn drive(
    input: Res<StepInput>,
    time: Res<Time>,
    active: Res<ActiveCar>,
    aids: Res<DriverAids>,
    mut cars: Query<&mut Car>,
    mut refused: MessageWriter<ReverseRefused>,
) {
    let idle = DriverInput {
        away: true,
        ..default()
    };
    for mut car in &mut cars {
        let input = if car.index == active.0 {
            car.traction_control = aids.traction_control;
            car.abs = aids.abs;
            &input.input
        } else {
            &idle
        };
        if car.drive(input, time.delta_secs()) {
            refused.write(ReverseRefused);
        }
    }
}

impl Car {
    /// The driver's `input` for a step of `dt` to the car's controls, by the car's rules:
    /// Q/E (or the paddles, or an H-shifter) change gear, the ignition button held runs
    /// the starter and pressed while running switches the engine off, keys steer
    /// rate-limited, the handbrake button works the parking brake (`parking_brake_rule`).
    /// Says whether reverse was asked for while rolling (and refused).
    pub fn drive(&mut self, input: &DriverInput, dt: f32) -> bool {
        if input.away {
            self.controls = Controls {
                brake: 1.0,
                handbrake: 1.0,
                ..default()
            };
            self.drivetrain.grinding = None;
            self.drivetrain.starter = false;
            return false;
        }
        let speed = self.forward_speed();
        let Car {
            setup,
            corners,
            drivetrain,
            controls: c,
            parking_brake,
            switching_off,
            ..
        } = self;
        // The ignition button: held, it runs the starter; pressed with the engine running,
        // it switches it off (and holding on after that doesn't crank).
        if !input.ignition_held {
            *switching_off = false;
        }
        c.clutch = input.clutch;
        if input.ignition && drivetrain.running {
            drivetrain.switch_off();
            *switching_off = true;
        }
        drivetrain.starter = input.ignition_held && !*switching_off;
        let gear = drivetrain.selected_gear();
        let shift = input.shift;
        let shares = setup.engine.drive.shares();
        let wheel_omega = (0..4).map(|i| shares[i] * corners[i].omega).sum();
        if let Some(gate) = input.shifter {
            drivetrain.select_gate(&setup.engine, gate, speed, wheel_omega);
        } else {
            drivetrain.grinding = None;
        }
        if input.shifter.is_none() && shift != 0 {
            // Q/E or the paddles shift down/up.
            drivetrain.shift_by(&setup.engine, shift, wheel_omega, speed);
        }
        let wants_reverse = match input.shifter {
            Some(gate) => gate < 0,
            None => shift < 0 && gear == 0,
        };
        let refused =
            wants_reverse && drivetrain.selected_gear() >= 0 && speed.abs() >= REVERSE_SPEED;
        let (go, stop) = (input.throttle, input.brake);
        (c.throttle, c.brake) = if go > 0.0 { (go, stop) } else { (0.0, stop) };
        let pull;
        (*parking_brake, pull) = parking_brake_rule(
            *parking_brake,
            input.handbrake_toggle,
            input.handbrake_held,
            go,
            speed,
        );
        // A lever works directly, on top of it.
        c.handbrake = pull.max(input.handbrake.unwrap_or(0.0));
        if input.held {
            c.brake = 1.0;
            c.handbrake = 1.0;
        }
        if let Some(steer) = input.steer {
            // A steering wheel turns the front wheels directly, at a fixed ratio.
            c.steer = steer;
        } else {
            // Keys steer less at speed, and are rate-limited like turning a real wheel.
            // Turning in is slow so a tap only nudges the wheels; letting go (or steering
            // the other way) brings them back to the middle faster.
            let target = input.steer_keys / (1.0 + speed.abs() * 0.04);
            let towards_middle = (target - c.steer) * c.steer < 0.0;
            let rate = if towards_middle {
                KEY_STEER_RETURN_RATE
            } else {
                KEY_STEER_RATE
            };
            let max_delta = rate * dt;
            c.steer += (target - c.steer).clamp(-max_delta, max_delta);
        }
        refused
    }
}

/// Below this speed (m/s) a handbrake press pulls the parking brake on.
pub const PARKING_SPEED: f32 = 0.5;

/// The handbrake button (Space) and the gas on the parking brake: pressed while it's on,
/// it comes off; pressed standing, it goes on; pressed while driving, it only brakes while
/// held (asked for by the user, 2026-11-24). Any gas releases it, whatever the gear or
/// clutch. Gives the parking brake and how hard the handbrake pulls.
pub fn parking_brake_rule(
    on: bool,
    pressed: bool,
    held: bool,
    throttle: f32,
    speed: f32,
) -> (bool, f32) {
    let mut on = on;
    if pressed {
        on = !on && speed.abs() < PARKING_SPEED;
    }
    if throttle > 0.0 {
        on = false;
    }
    (on, if on || held { 1.0 } else { 0.0 })
}

fn reset_car(
    wheel: Res<WheelInput>,
    active: Res<ActiveCar>,
    terrain: Res<Terrain>,
    cars: Query<(Entity, &Car)>,
    (away, rule): (Res<Away>, Res<ResetRule>),
    mut rescue: ResMut<Rescue>,
) {
    if wheel.reset && !away.0 && !rescue.busy() {
        for (entity, car) in cars.iter().filter(|(_, car)| car.index == active.0) {
            match *rule {
                ResetRule::Road => {
                    let (p, yaw) = car.road_spot(&terrain);
                    rescue.call(entity, car, &terrain, p, yaw);
                }
                ResetRule::Spot(p, yaw) => rescue.call(entity, car, &terrain, p, yaw),
                ResetRule::Never => {}
            }
        }
    }
}

pub fn sync_transforms(mut cars: Query<(&Car, &mut Transform)>) {
    for (car, mut transform) in &mut cars {
        transform.translation = car.body.position;
        transform.rotation = car.body.rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_road_spot_is_on_the_nearest_road() {
        let terrain = Terrain::baked();
        let (spawn, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        // 40 m off the road to the side, lying on its roof.
        let side = Vec2::new(yaw.cos(), -yaw.sin());
        let off = spawn + 40.0 * side;
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, off.x, off.y, yaw);
        car.body.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(3.0);
        let (p, yaw) = car.road_spot(&terrain);
        car.place(&terrain, p, yaw);
        let p = car.body.position.xz();
        // In the lane for the way it faces.
        let facing = (car.body.rotation * Vec3::NEG_Z).xz();
        let (lane, _) = terrain.drive_spot(p, facing).unwrap();
        assert!(
            p.distance(lane) < 0.1,
            "{p} is {} m off the lane",
            p.distance(lane)
        );
        assert!(p.distance(off) < 40.0, "{p} isn't the nearest road");
        assert!((car.body.rotation * Vec3::Y).y > 0.99, "not upright");
        assert!(car.parking_brake);
    }

    /// Drives `car` for `seconds` (from `from`) on `terrain` with the controls changing all
    /// the time: gas, brakes (ABS), steering, shifts up and down, the handbrake.
    fn drive_about(car: &mut Car, terrain: &Terrain, from: f32, seconds: f32) {
        let dt = 1.0 / crate::sim::CHASSIS_HZ as f32;
        let steps = (seconds / dt) as usize;
        for i in 0..steps {
            let t = from + i as f32 * dt;
            let input = DriverInput {
                steer: Some(0.4 * (t * 0.7).sin()),
                throttle: (0.5 + 0.5 * (t * 0.5).sin()).clamp(0.0, 1.0),
                brake: (2.0 * ((t * 0.9 + 3.0).sin() - 0.7)).max(0.0),
                handbrake_held: (t % 7.0) < 0.3,
                // Up at 1.5 s, 3 s, down at 6.5 s, every 8 s.
                shift: match ((t % 8.0) / dt) as usize {
                    180 | 360 => 1,
                    780 => -1,
                    _ => 0,
                },
                ..default()
            };
            car.drive(&input, dt);
            crate::sim::step_car(car, terrain, dt);
        }
    }

    #[test]
    fn a_loaded_car_drives_on_as_the_saved_one() {
        use crate::snapshot::{load, save};
        let terrain = Terrain::baked();
        let (p, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let setup = CarSetup::factory(5, 0);
        let mut car = Car::spawn_at(0, setup.clone(), &terrain, p.x, p.y, yaw);
        car.drivetrain.shift_to(&car.setup.engine, 1);
        drive_about(&mut car, &terrain, 0.0, 4.0);
        let bytes = save(&car);
        // A car of the same setup somewhere else, fresh from the factory.
        let mut loaded = Car::spawn_at(0, setup, &terrain, p.x + 50.0, p.y, 0.0);
        assert!(load(&mut loaded, &bytes));
        assert_eq!(save(&loaded), bytes);
        drive_about(&mut car, &terrain, 4.0, 6.0);
        drive_about(&mut loaded, &terrain, 4.0, 6.0);
        assert_eq!(save(&loaded), save(&car), "the loaded car went its own way");
        assert!(car.fuel.distance > 15.0, "drove {} m", car.fuel.distance);
        println!("{} bytes", bytes.len());
    }

    #[test]
    fn a_snapshot_of_another_gearbox_is_refused() {
        use crate::snapshot::{load, save};
        let terrain = Terrain::flat();
        // The van's in fifth, a gear the city car's box has; then sixth.
        let mut van = Car::spawn_at(0, CarSetup::factory(12, 0), &terrain, 0.0, 0.0, 0.0);
        let mut minim = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
        let top = minim.setup.engine.top_gear();
        van.drivetrain.gear = top;
        assert!(load(&mut minim, &save(&van)));
        van.drivetrain.gear = top + 1;
        assert!(!load(&mut minim, &save(&van)));
        assert!(!load(&mut minim, &save(&van)[..20]), "cut short");
    }

    #[test]
    fn inputs_read_back_as_sent() {
        let input = DriverInput {
            steer: Some(-0.123456),
            steer_keys: -1.0,
            throttle: 0.7,
            brake: 1.0,
            clutch: Some(0.25),
            handbrake: None,
            handbrake_toggle: true,
            handbrake_held: false,
            shifter: Some(-1),
            shift: 2,
            ignition: false,
            ignition_held: true,
            away: false,
            held: true,
        };
        let sent = input.sent();
        assert!((sent.steer.unwrap() - input.steer.unwrap()).abs() < 1e-4);
        assert!((sent.throttle - 0.7).abs() < 1e-4);
        assert_eq!(sent.sent(), sent, "sent twice, the same");
        let bytes = encode(&sent).len();
        assert!(bytes <= 14, "{bytes} bytes");
        assert_eq!(
            DriverInput::default().sent(),
            DriverInput::default(),
            "nothing pressed"
        );
    }

    #[test]
    fn the_handbrake_parks_standing_brakes_driving_and_gas_releases_it() {
        // (on, pressed, held, throttle, speed) -> (on, pull)
        let rule = parking_brake_rule;
        // Standing, a press pulls it on; pressed again, it comes off.
        assert_eq!(rule(false, true, true, 0.0, 0.0), (true, 1.0));
        assert_eq!(rule(true, false, false, 0.0, 0.0), (true, 1.0));
        assert!(!rule(true, true, true, 0.0, 0.0).0);
        // Driving, it brakes only while held.
        assert_eq!(rule(false, true, true, 0.0, 15.0), (false, 1.0));
        assert_eq!(rule(false, false, true, 0.0, 15.0), (false, 1.0));
        assert_eq!(rule(false, false, false, 0.0, 15.0), (false, 0.0));
        // On while rolling, a press releases it.
        assert_eq!(rule(true, true, false, 0.0, 5.0), (false, 0.0));
        // Any gas releases it.
        assert_eq!(rule(true, false, false, 0.01, 0.0), (false, 0.0));
        assert_eq!(rule(true, false, false, 1.0, 0.0), (false, 0.0));
    }

    #[test]
    fn driver_aids_read_back() {
        let aids = DriverAids {
            traction_control: false,
            abs: true,
        };
        assert_eq!(DriverAids::from_text(&aids.to_text()), aids);
        // Broken and unknown lines (the gearbox mode there once was) keep the defaults.
        assert_eq!(
            DriverAids::from_text("abs maybe\ntraction_control false\nautomatic true\n"),
            aids
        );
    }
}
