//! Car setups, spawning and driver input.

use bevy::prelude::*;

use crate::chassis::{CLUTCHES, CarBuild, Chassis, DEFAULT_CHASSIS, SUSPENSIONS};
use crate::input::WheelInput;
use crate::physics::collision::Obb;
use crate::physics::drivetrain::{Differential, Drive, Drivetrain, EngineSetup};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::parts::Parts;
use crate::physics::fuel::{self, Fuel, Tank};
use crate::physics::rigid_body::RigidBody;
use crate::physics::suspension::{ArmGeometry, AxleSetup, Corner};
use crate::physics::tire::{COMPOUNDS, TireParams};
use crate::storage::Folder;
use crate::terrain::Terrain;

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
        let (_, ride, damping, camber) = SUSPENSIONS[build.suspension];
        let compound = COMPOUNDS[build.tires];
        let grip = chassis.grip * compound.grip;
        let axle = |i: usize| {
            let radius = chassis.tire_radius[i];
            let share = if i == 0 {
                front_share
            } else {
                1.0 - front_share
            };
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
            geometry.static_camber = (chassis.camber[i] - camber).to_radians();
            // Toe-in at the rear keeps the car stable, especially off throttle.
            geometry.static_toe = if i == 0 { 0.0 } else { 0.25f32.to_radians() };
            let stock = TireParams::default();
            let unsprung_mass = chassis.unsprung[i];
            let mut axle = AxleSetup {
                geometry,
                spring_rate: 0.0,
                damping: 0.0,
                unsprung_mass,
                wheel_inertia: 0.45 * unsprung_mass * radius * radius,
                tire: TireParams {
                    radius,
                    compound,
                    mu: grip,
                    // The tires are sized for the car: their friction is `grip` at rest
                    // (warm and new).
                    nominal_load: (laden * share / 2.0 + unsprung_mass) * 9.81,
                    ..stock
                },
            };
            axle.tune(
                chassis.ride[i] * ride,
                chassis.damping * damping,
                laden * share / 2.0,
            );
            axle
        };

        let mass = laden + 2.0 * (chassis.unsprung[0] + chassis.unsprung[1]);
        let radius = (chassis.tire_radius[0] + chassis.tire_radius[1]) / 2.0;
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
        let mut hull = body_box(chassis);
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
            brake_torque: chassis.brakes * chassis.grip * mass * 9.81 * radius,
            brake_bias: brake_bias.min(0.9),
            handbrake_torque: 2.0 * mass,
            drag_area: chassis.drag_area,
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
    pub fn corner_mass_with(&self, front: bool, litres: f32) -> f32 {
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

/// The box around a chassis' body (and its wheels, where they stand out), in the body's
/// design frame: its origin on the ground midway between the axles, forward -z.
pub fn body_box(chassis: &Chassis) -> Obb {
    let body = &chassis.body;
    let wheels = chassis.wheels();
    let top = (body.deck.iter().map(|p| p[1]))
        .chain(body.cabin.iter().map(|p| p[1]))
        .fold(0.0, f32::max);
    let (nose, tail) = (body.z(0.0, &wheels), body.z(body.length, &wheels));
    let half_width = (0..2)
        .map(|i| (chassis.track[i] + chassis.tire_width[i]) / 2.0)
        .fold(body.width / 2.0, f32::max);
    Obb {
        center: Vec3::new(0.0, (body.floor + top) / 2.0, (nose + tail) / 2.0),
        axes: [Vec3::X, Vec3::Y, Vec3::Z],
        half: Vec3::new(half_width, (top - body.floor) / 2.0, (tail - nose) / 2.0),
    }
}

impl Default for CarSetup {
    /// The coupé with its base engine.
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

#[derive(Component)]
pub struct Car {
    /// Which car it is (see `ActiveCar`).
    pub index: usize,
    pub setup: CarSetup,
    pub body: RigidBody,
    pub corners: [Corner; 4],
    pub drivetrain: Drivetrain,
    pub controls: Controls,
    /// Torque of the front tires about the steering axes (N·m, positive turns the wheels
    /// left), for force feedback.
    pub steering_torque: f32,
    /// Traction control: whether it's on, and the share of the driver's throttle it
    /// lets through right now.
    pub traction_control: bool,
    pub traction: f32,
    /// ABS: whether it's on, and the share of the brake pedal it lets through to each
    /// wheel right now.
    pub abs: bool,
    pub abs_pressure: [f32; 4],
    /// The fuel in the tank, and the share of the tank the driver fills it to (on a reset).
    pub fuel: Fuel,
    pub fill: f32,
    /// How hard the body has hit houses, trees and other cars since this was last taken:
    /// the speed the knocks changed (m/s), added up.
    pub knocks: f32,
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
        let position = Vec3::new(x, terrain.sample(x, z).0 + height, z);
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
            steering_torque: 0.0,
            traction_control: true,
            traction: 1.0,
            abs: true,
            abs_pressure: [1.0; 4],
            fuel,
            fill: 1.0,
            knocks: 0.0,
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

    /// Fills the tank to `fill` (a share of it) now and at every reset.
    pub fn refuel(&mut self, fill: f32) {
        self.fill = fill.clamp(0.0, 1.0);
        self.fuel = Fuel::new(self.fill * self.setup.tank.litres);
        self.load_fuel();
    }

    /// Places the car upright, at rest, just above the ground at its current spot.
    fn reset(&mut self, terrain: &Terrain) {
        let p = self.body.position;
        self.place(terrain, p.xz(), heading(self.body.rotation));
    }

    /// Puts the car at rest at (x, z) in the xz plane, facing along `yaw`, keeping its
    /// setup and the driver's settings.
    pub fn place(&mut self, terrain: &Terrain, p: Vec2, yaw: f32) {
        let mut placed = Self::spawn_at(self.index, self.setup.clone(), terrain, p.x, p.y, yaw);
        placed.keep_settings(self);
        placed.refuel(self.fill);
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
        car.keep_settings(self);
        if same_engine {
            car.drivetrain.engine = self.drivetrain.engine;
        }
        let (new, old) = (&car.setup.build, &self.setup.build);
        let same_tires = new.chassis == old.chassis && new.tires == old.tires;
        for (corner, old) in car.corners.iter_mut().zip(&self.corners) {
            corner.omega = old.omega;
            if same_tires {
                corner.tread = old.tread;
            }
        }
        car.fill = self.fill;
        car.fuel = self.fuel;
        car.fuel.fit(&car.setup.tank);
        car.load_fuel();
        car
    }

    /// Takes the driver's settings (gearbox mode, traction control, ABS) from the car this
    /// one replaces. (The active car gets them from `DriverAids` anyway.)
    pub fn keep_settings(&mut self, old: &Car) {
        self.drivetrain.automatic = old.drivetrain.automatic;
        self.traction_control = old.traction_control;
        self.abs = old.abs;
    }

    /// Brings every tread to its compound's best temperature, as after a few laps.
    #[cfg(test)]
    pub fn warm_tires(&mut self) {
        for corner in &mut self.corners {
            corner.tread.temperature = self.setup.axle(corner.front).tire.compound.best;
        }
    }

    pub fn forward_speed(&self) -> f32 {
        self.body.velocity.dot(self.body.rotation * Vec3::NEG_Z)
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

const SETTINGS: Folder = Folder {
    cache: false,
    name: "settings",
    extension: "cfg",
    web_prefix: "car-sim-settings:",
};

/// The driver's choice of aids (gearbox mode, traction control, ABS), the same for every
/// car and kept between runs (`~/.config/car-sim/settings/aids.cfg`): `drive` hands it
/// to the active car.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct DriverAids {
    pub automatic: bool,
    pub traction_control: bool,
    pub abs: bool,
}

impl Default for DriverAids {
    fn default() -> Self {
        Self {
            automatic: false,
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
            "automatic {}\ntraction_control {}\nabs {}\n",
            self.automatic, self.traction_control, self.abs
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
                "automatic" => aids.automatic = on,
                "traction_control" => aids.traction_control = on,
                "abs" => aids.abs = on,
                _ => {}
            }
        }
        aids
    }

    /// Checkboxes for them all; saves a change.
    pub fn show(&mut self, ui: &mut bevy_egui::egui::Ui) {
        let before = *self;
        ui.checkbox(&mut self.automatic, "Automatic (G)");
        ui.checkbox(&mut self.traction_control, "Traction control")
            .on_hover_text("Eases the throttle while a driven wheel spins");
        ui.checkbox(&mut self.abs, "ABS")
            .on_hover_text("Eases the brake of a wheel that's about to lock");
        if *self != before {
            self.save();
        }
    }
}

/// Driving the active car, and its entity following the simulation. Which cars there are
/// is up to the app.
pub struct CarPlugin;

impl Plugin for CarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveCar>()
            .insert_resource(DriverAids::load())
            .add_systems(Update, (drive, sync_transforms).chain());
    }
}

/// Starts in the coupé with its base engine, on the road at the map's spawn point.
pub struct DefaultCarPlugin;

impl Plugin for DefaultCarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_default_car);
    }
}

/// R (or the wheel's reset button) puts the active car back on its wheels where it is, as
/// new: refuelled to the fuel load, fresh tires, a repaired engine.
pub struct ResetPlugin;

impl Plugin for ResetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, reset_car.after(drive).before(sync_transforms));
    }
}

fn spawn_default_car(mut commands: Commands, terrain: Res<Terrain>) {
    // On the road at the spawn point.
    let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
    let car = Car::spawn_at(0, CarSetup::default(), &terrain, p.x, p.y, yaw);
    commands.spawn((Transform::from_translation(car.body.position), car));
}

/// Keyboard and wheel to controls for the active car; any other car holds all brakes. G
/// switches between manual and automatic gearboxes, I starts a stalled engine.
pub fn drive(
    keys: Res<ButtonInput<KeyCode>>,
    wheel: Res<WheelInput>,
    time: Res<Time>,
    active: Res<ActiveCar>,
    mut aids: ResMut<DriverAids>,
    mut cars: Query<&mut Car>,
) {
    let pressed = |a, b| keys.pressed(a) || keys.pressed(b);
    if keys.just_pressed(KeyCode::KeyG) {
        aids.automatic ^= true;
        aids.save();
    }
    for mut car in &mut cars {
        if car.index != active.0 {
            car.controls = Controls {
                brake: 1.0,
                handbrake: 1.0,
                ..default()
            };
            continue;
        }
        car.drivetrain.automatic = aids.automatic;
        car.traction_control = aids.traction_control;
        car.abs = aids.abs;
        let speed = car.forward_speed();
        let forward = pressed(KeyCode::KeyW, KeyCode::ArrowUp);
        let back = pressed(KeyCode::KeyS, KeyCode::ArrowDown);
        let steer_target = pressed(KeyCode::KeyA, KeyCode::ArrowLeft) as i32 as f32
            - pressed(KeyCode::KeyD, KeyCode::ArrowRight) as i32 as f32;

        let Car {
            setup,
            corners,
            drivetrain,
            controls: c,
            ..
        } = &mut *car;
        c.clutch = wheel.clutch;
        if keys.just_pressed(KeyCode::KeyI) || wheel.ignition {
            drivetrain.start(&setup.engine);
        }
        let gear = drivetrain.selected_gear();
        let shift = keys.just_pressed(KeyCode::KeyE) as i8 - keys.just_pressed(KeyCode::KeyQ) as i8
            + wheel.shift_up as i8
            - wheel.shift_down as i8;
        let (go, stop) = if drivetrain.automatic {
            // W drives forward and S backward: each brakes while rolling the other way
            // and selects first or reverse once (nearly) stopped.
            let reverse = gear < 0;
            let (go, stop) = if reverse {
                (back, forward)
            } else {
                (forward, back)
            };
            let along = if reverse { -speed } else { speed };
            if stop && !go && along < 1.0 {
                drivetrain.shift_to(&setup.engine, if reverse { 1 } else { -1 });
            }
            // Shifting down or up at a stop selects reverse or drive.
            if speed.abs() < 1.0 && shift != 0 {
                drivetrain.shift_to(&setup.engine, if shift < 0 { -1 } else { 1 });
            }
            (go, stop)
        } else if let Some(gate) = wheel.shifter {
            drivetrain.select_gate(&setup.engine, gate, speed);
            (forward, back)
        } else {
            // Q/E or the paddles shift down/up.
            if shift != 0 {
                let shares = setup.engine.drive.shares();
                let wheel_omega = (0..4).map(|i| shares[i] * corners[i].omega).sum();
                drivetrain.shift_by(&setup.engine, shift, wheel_omega, speed);
            }
            (forward, back)
        };
        // The pedals add to the keys; they work the same in both gearbox modes.
        let go = (go as u8 as f32).max(wheel.throttle.unwrap_or(0.0));
        let stop = (stop as u8 as f32).max(wheel.brake.unwrap_or(0.0));
        (c.throttle, c.brake) = if go > 0.0 {
            (go, stop)
        } else if stop > 0.0 {
            (0.0, stop)
        } else if speed.abs() < 2.0 {
            // Hill hold: without input, keep a (nearly) stopped car from rolling away.
            (0.0, 1.0)
        } else {
            (0.0, 0.0)
        };
        c.handbrake =
            (keys.pressed(KeyCode::Space) as u8 as f32).max(wheel.handbrake.unwrap_or(0.0));
        if let Some(steer) = wheel.steer {
            // A steering wheel turns the front wheels directly, at a fixed ratio.
            c.steer = steer;
        } else {
            // Keys steer less at speed, and are rate-limited like turning a real wheel.
            let target = steer_target / (1.0 + speed.abs() * 0.04);
            let max_delta = 3.0 * time.delta_secs();
            c.steer += (target - c.steer).clamp(-max_delta, max_delta);
        }
    }
}

fn reset_car(
    keys: Res<ButtonInput<KeyCode>>,
    wheel: Res<WheelInput>,
    active: Res<ActiveCar>,
    terrain: Res<Terrain>,
    mut cars: Query<&mut Car>,
) {
    if keys.just_pressed(KeyCode::KeyR) || wheel.reset {
        for mut car in cars.iter_mut().filter(|car| car.index == active.0) {
            car.reset(&terrain);
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
    fn driver_aids_read_back() {
        let aids = DriverAids {
            automatic: true,
            traction_control: false,
            abs: true,
        };
        assert_eq!(DriverAids::from_text(&aids.to_text()), aids);
        // Broken lines keep the defaults.
        assert_eq!(
            DriverAids::from_text("abs maybe\ntraction_control false\nautomatic true\n"),
            aids
        );
    }
}
