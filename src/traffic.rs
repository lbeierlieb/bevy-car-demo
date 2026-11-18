//! Traffic: cars driving the roads around the player's car.
//!
//! They aren't simulated like the player's car. A driver follows a route through the road
//! network (`network.rs`), picking a way at random at each junction; the front axle runs
//! along the lane, the rear axle is dragged after it, and the body sits on the ground
//! under the wheels. The speed follows the limits, slowing for bends and junctions
//! ahead, and keeps a distance to whatever is in the lane ahead (the intelligent driver
//! model). Any knock against another car (or the player's) ends the drive: the car is a
//! loose rigid body on its box from then on, sliding and tumbling to a stop. Cars come
//! and go out of sight around the player.

pub mod network;

use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use crate::body::Wheels;
use crate::car::{ActiveCar, Car, body_box};
use crate::chassis::{CHASSIS, PAINTS};
use crate::physics::collision::{self, Contact, GROUND, OBSTACLE, Obb};
use crate::physics::rigid_body::RigidBody;
use crate::terrain::Terrain;
use network::{Leg, Network};

const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// How many traffic cars there are around the player by default.
const DEFAULT_CARS: usize = 20;
pub const MOST_CARS: usize = 60;
/// Cars appear this far from the player (closer only when the game starts, before
/// anything is seen), and go once they're farther than this, or this when crashed (m).
const SPAWN_NEAR: f32 = 260.0;
const SPAWN_FIRST: f32 = 50.0;
const SPAWN_FAR: f32 = 550.0;
const DESPAWN: f32 = 700.0;
const DESPAWN_CRASHED: f32 = 350.0;
/// No car appears closer than this to another (m).
const SPAWN_GAP: f32 = 30.0;
/// Where cars may appear: a spot every so far along the roads (m).
const SPAWN_STEP: f32 = 30.0;

/// How far ahead a driver plans the route and looks for bends and limits, and for cars
/// to follow (m).
const LOOKAHEAD: f32 = 200.0;
const FOLLOW_RANGE: f32 = 100.0;
/// Drivers' acceleration, comfortable braking, the hardest braking (m/s²), the time gap
/// they keep, and the gap at a standstill (s, m): the intelligent driver model.
const ACCELERATION: f32 = 1.4;
const COMFORT_BRAKING: f32 = 2.0;
const HARDEST_BRAKING: f32 = 8.0;
/// How quickly a driver gets back down to the speed that's allowed when over it (s).
const TRACKING: f32 = 0.3;
const TIME_GAP: f32 = 1.4;
const STANDSTILL_GAP: f32 = 2.5;
/// How hard drivers take bends (sideways, m/s²), and over how far they judge a bend (m).
const CORNERING: f32 = 2.5;
const CHORD: f32 = 5.0;
/// The path has a point at least this often, for the limits along it (m).
const SAMPLE: f32 = network::LIMIT_STEP / 2.0;
/// Corners at junctions are rounded over this much of each lane (m).
const CORNER: f32 = 6.0;
/// Something reaching nearer the lane than this past the car's side is in the way (m);
/// a car coming the other way only with its middle this near (not one in the other
/// lane, even on a narrow road).
const IN_LANE: f32 = 0.2;
const ONCOMING: f32 = 1.0;
/// Some drivers keep below the limit, some go a little over.
const KEEN: (f32, f32) = (0.88, 1.1);
/// How often each chassis turns up in traffic (in `CHASSIS` order).
const COMMON: [f32; 13] = [
    4.0, 3.0, 0.7, 3.0, 4.0, 0.8, 1.0, 0.4, 0.8, 0.2, 0.15, 2.0, 3.0,
];
/// Masses of an engine and gearbox, added to a chassis' (kg).
const DRIVETRAIN_MASS: f32 = 230.0;
/// A crashed car stops being simulated once it's been this still for this long (m/s,
/// rad/s, s).
const STILL: f32 = 0.05;
const SETTLE_TIME: f32 = 2.0;
/// The rim's radius relative to the tire's.
const RIM: f32 = 0.64;

/// How many traffic cars drive around the player (the panel's traffic density).
#[derive(Resource)]
pub struct Traffic {
    pub cars: usize,
}

impl Default for Traffic {
    fn default() -> Self {
        Self { cars: DEFAULT_CARS }
    }
}

pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Traffic>()
            .add_systems(Startup, |mut commands: Commands, terrain: Res<Terrain>| {
                let network = Network::new(&terrain);
                commands.insert_resource(Spawner::new(&network));
                commands.insert_resource(network);
            })
            .add_systems(Update, (populate, pose).chain())
            .add_systems(FixedUpdate, step.after(crate::sim::simulate));
    }
}

/// A traffic car.
#[derive(Component)]
pub struct TrafficCar {
    /// Its reference point is the body's design origin (on the ground midway between the
    /// axles); the centre of mass is above it.
    pub body: RigidBody,
    /// Its box in the body's frame, down to the ground (it stands on it once crashed).
    pub hull: Obb,
    wheels: Wheels,
    /// Whoever drives it, until it crashes.
    driver: Option<Driver>,
    /// How far the wheels have rolled (m), and the front wheels' steering angle (rad).
    rolled: f32,
    steer: f32,
    /// How long it's been still since it crashed (s).
    still: f32,
}

struct Driver {
    /// The legs ahead, the one it's on first.
    route: VecDeque<Leg>,
    /// Where the path starts on the first leg's lane: past the corner into it (m).
    start: f32,
    /// How far the front axle is along the path (m).
    along: f32,
    /// The rear axle, dragged after the front one.
    rear: Vec2,
    speed: f32,
    /// Share of the limit this driver likes to go.
    keen: f32,
    rng: Rng,
}

/// The way ahead of a driver, along the lanes of its route with the corners at the
/// junctions rounded: points, their distance from the start, and the limit at each.
#[derive(Default)]
struct Path {
    points: Vec<Vec2>,
    at: Vec<f32>,
    limits: Vec<f32>,
    /// Where the second leg begins (past the corner into it), if there's one.
    seam: Option<f32>,
}

impl Path {
    fn clear(&mut self) {
        self.points.clear();
        self.at.clear();
        self.limits.clear();
        self.seam = None;
    }

    fn push(&mut self, p: Vec2, limit: f32) {
        if let Some(&last) = self.points.last() {
            let step = last.distance(p);
            if step < 1e-3 {
                return;
            }
            self.at.push(self.at.last().unwrap() + step);
        } else {
            self.at.push(0.0);
        }
        self.points.push(p);
        self.limits.push(limit);
    }

    fn length(&self) -> f32 {
        self.at.last().copied().unwrap_or(0.0)
    }

    /// The point `d` along the path (clamped to it), and the direction there.
    fn point(&self, d: f32) -> (Vec2, Vec2) {
        let n = self.points.len();
        if n < 2 {
            return (self.points.first().copied().unwrap_or(Vec2::ZERO), Vec2::Y);
        }
        let i = self.at.partition_point(|&a| a <= d).clamp(1, n - 1) - 1;
        let (a, b) = (self.points[i], self.points[i + 1]);
        let span = (self.at[i + 1] - self.at[i]).max(1e-6);
        let t = ((d - self.at[i]) / span).clamp(0.0, 1.0);
        let direction = (b - a).normalize_or(Vec2::Y);
        // Beyond the ends, straight on.
        let beyond = if d < 0.0 {
            d
        } else {
            (d - self.length()).max(0.0)
        };
        (a.lerp(b, t) + direction * beyond, direction)
    }

    /// Builds the path along `route`, from `start` along the first leg's lane.
    fn build(&mut self, network: &Network, route: &VecDeque<Leg>, start: f32) {
        self.clear();
        let mut from = start;
        for (i, &leg) in route.iter().enumerate() {
            let lane = network.lane(leg);
            let next = route.get(i + 1).copied();
            let to = lane.length() - next.map_or(0.0, |next| corner(network, leg, next));
            // The lane's points, and more on long straights so that no change of the
            // limit goes unseen.
            let mut s = from;
            self.push(lane.point(s), network.limit(leg, s));
            for &a in lane.at.iter().filter(|&&a| a > from && a < to).chain([&to]) {
                while a - s > SAMPLE {
                    s += SAMPLE;
                    self.push(lane.point(s), network.limit(leg, s));
                }
                s = a;
                self.push(lane.point(s), network.limit(leg, s));
            }
            let Some(next) = next else { break };
            // A quadratic curve from the end of this lane to the start of the next one,
            // bent towards where their directions meet (the midway point when they
            // don't, going straight on or turning round).
            let next_lane = network.lane(next);
            let into = corner(network, leg, next);
            let (p0, p2) = (lane.point(to), next_lane.point(into));
            let (u, v) = (lane.direction(to), next_lane.direction(into));
            let t = (p2 - p0).perp_dot(v) / u.perp_dot(v);
            let p1 = if u.perp_dot(v).abs() > 0.2 && t > 0.0 && t < 3.0 * CORNER {
                p0 + u * t
            } else {
                (p0 + p2) / 2.0
            };
            let limit = network.limit(leg, to).min(network.limit(next, into));
            for k in 1..6 {
                let t = k as f32 / 6.0;
                let p = p0 * (1.0 - t) * (1.0 - t) + p1 * 2.0 * t * (1.0 - t) + p2 * t * t;
                self.push(p, limit);
            }
            self.push(p2, network.limit(next, into));
            if i == 0 {
                self.seam = Some(self.length());
            }
            from = into;
        }
    }
}

/// How much of each lane the corner from `leg` into `next` takes up (m).
fn corner(network: &Network, leg: Leg, next: Leg) -> f32 {
    let a = network.lane(leg).length();
    let b = network.lane(next).length();
    CORNER.min(0.4 * a).min(0.4 * b)
}

/// A small xorshift generator: each driver has its own.
#[derive(Clone, Copy)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, (low, high): (f32, f32)) -> f32 {
        low + (high - low) * self.next()
    }

    /// An index into `weights`, picked in proportion to them.
    fn pick(&mut self, weights: impl Iterator<Item = f32> + Clone) -> usize {
        let total: f32 = weights.clone().sum();
        let mut x = self.next() * total;
        let mut last = 0;
        for (i, w) in weights.enumerate() {
            if x < w {
                return i;
            }
            x -= w;
            last = i;
        }
        last
    }
}

impl Driver {
    /// Plans the route at least `LOOKAHEAD` past the front axle: at each junction a way
    /// at random, wide roads more likely, turning round at dead ends.
    fn plan(&mut self, network: &Network) {
        let mut ahead = -self.start - self.along;
        for &leg in &self.route {
            ahead += network.lane(leg).length();
        }
        while ahead < LOOKAHEAD {
            let last = *self.route.back().unwrap();
            let ways: Vec<Leg> = network.next(last).collect();
            let next = if ways.is_empty() {
                Leg {
                    edge: last.edge,
                    forward: !last.forward,
                }
            } else {
                let weights = ways
                    .iter()
                    .map(|way| network.edges[way.edge as usize].width.powi(2));
                ways[self.rng.pick(weights)]
            };
            ahead += network.lane(next).length();
            self.route.push_back(next);
        }
    }
}

/// Where cars may appear, and the meshes and materials they're made of.
#[derive(Resource)]
struct Spawner {
    /// (leg, distance along its lane, position) every `SPAWN_STEP` along the roads.
    spots: Vec<(Leg, f32, Vec2)>,
    rng: Rng,
    /// The first cars may appear nearer, before anything is seen.
    started: bool,
    meshes: Option<Meshes>,
}

struct Meshes {
    /// A body for each chassis and paint, made when first needed.
    bodies: Vec<Option<Handle<Mesh>>>,
    wheel: Handle<Mesh>,
    bar: Handle<Mesh>,
    paint: Handle<StandardMaterial>,
    rubber: Handle<StandardMaterial>,
    alloy: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

impl Spawner {
    fn new(network: &Network) -> Self {
        let mut spots = Vec::new();
        for e in 0..network.edges.len() {
            for forward in [true, false] {
                let leg = Leg {
                    edge: e as u32,
                    forward,
                };
                let lane = network.lane(leg);
                let mut s = SPAWN_STEP / 2.0;
                while s < lane.length() {
                    spots.push((leg, s, lane.point(s)));
                    s += SPAWN_STEP;
                }
            }
        }
        Self {
            spots,
            rng: Rng(0x2545_f491_4f6c_dd1d),
            started: false,
            meshes: None,
        }
    }
}

/// One of the wheels' parts, posed from its car.
#[derive(Component)]
struct WheelPart {
    corner: usize,
    kind: WheelKind,
}

#[derive(Clone, Copy)]
enum WheelKind {
    Tire,
    Rim,
    Spoke,
}

/// Keeps the number of cars around the player at what's asked for: new ones appear out
/// of sight on the roads, ones left far behind go (and when there are too many, the
/// farthest).
#[allow(clippy::too_many_arguments)]
fn populate(
    mut commands: Commands,
    traffic: Res<Traffic>,
    network: Res<Network>,
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    mut spawner: ResMut<Spawner>,
    cars: Query<&Car>,
    others: Query<(Entity, &TrafficCar)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(player) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let spawner = &mut *spawner;
    let center = player.body.position.xz();
    let distance = |car: &TrafficCar| car.body.position.xz().distance(center);
    let mut count = 0;
    let mut farthest: Option<(Entity, f32)> = None;
    for (entity, car) in &others {
        let d = distance(car);
        let reach = if car.driver.is_some() {
            DESPAWN
        } else {
            DESPAWN_CRASHED
        };
        if d > reach {
            commands.entity(entity).despawn();
            continue;
        }
        count += 1;
        if farthest.is_none_or(|(_, far)| d > far) {
            farthest = Some((entity, d));
        }
    }
    if count > traffic.cars {
        if let Some((entity, _)) = farthest {
            commands.entity(entity).despawn();
        }
        return;
    }
    if count == traffic.cars {
        spawner.started = true;
        return;
    }

    // A spot on the roads in the ring around the player, clear of every other car.
    let near = if spawner.started {
        SPAWN_NEAR
    } else {
        SPAWN_FIRST
    };
    let taken: Vec<Vec2> = (others.iter().map(|(_, car)| car.body.position.xz()))
        .chain(cars.iter().map(|car| car.body.position.xz()))
        .collect();
    let free: Vec<usize> = (0..spawner.spots.len())
        .filter(|&i| {
            let p = spawner.spots[i].2;
            let d = p.distance(center);
            d > near
                && d < SPAWN_FAR
                && terrain.inside(p.x, p.y)
                && taken.iter().all(|t| t.distance(p) > SPAWN_GAP)
        })
        .collect();
    if free.is_empty() {
        spawner.started = true;
        return;
    }
    let rng = &mut spawner.rng;
    let (leg, s, _) = spawner.spots[free[(rng.next() * free.len() as f32) as usize % free.len()]];
    let chassis = rng.pick(COMMON.iter().copied());
    let paint = (rng.next() * PAINTS.len() as f32) as usize % PAINTS.len();
    let keen = rng.range(KEEN);
    let seed = rng.0;

    let parts = spawner.meshes.get_or_insert_with(|| {
        let mut flat = |color: Color, roughness: f32| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: roughness,
                ..default()
            })
        };
        Meshes {
            bodies: vec![None; CHASSIS.len() * PAINTS.len()],
            wheel: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(14).build()),
            bar: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            paint: flat(Color::WHITE, 0.45),
            rubber: flat(Color::srgb(0.06, 0.06, 0.06), 0.9),
            alloy: flat(Color::srgb(0.62, 0.64, 0.67), 0.4),
            dark: flat(Color::srgb(0.12, 0.12, 0.13), 0.6),
        }
    });
    let model = &CHASSIS[chassis];
    let wheels = model.wheels();
    let body_mesh = parts.bodies[chassis * PAINTS.len() + paint]
        .get_or_insert_with(|| {
            let [r, g, b] = PAINTS[paint].1;
            meshes.add(model.body.mesh(&wheels, Color::srgb(r, g, b)))
        })
        .clone();

    let car = TrafficCar::new(&network, &terrain, chassis, leg, s, keen, seed);
    commands
        .spawn((
            Transform::from_translation(car.body.position).with_rotation(car.body.rotation),
            Visibility::default(),
            car,
        ))
        .with_children(|parent| {
            parent.spawn((Mesh3d(body_mesh), MeshMaterial3d(parts.paint.clone())));
            for corner in 0..4 {
                for (kind, mesh, material) in [
                    (WheelKind::Tire, &parts.wheel, &parts.rubber),
                    (WheelKind::Rim, &parts.wheel, &parts.alloy),
                    (WheelKind::Spoke, &parts.bar, &parts.dark),
                ] {
                    parent.spawn((
                        WheelPart { corner, kind },
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                    ));
                }
            }
        });
}

impl TrafficCar {
    /// A car of `chassis` (in `CHASSIS`) with its front axle `s` along `leg`'s lane,
    /// driving off at a little under the limit.
    fn new(
        network: &Network,
        terrain: &Terrain,
        chassis: usize,
        leg: Leg,
        s: f32,
        keen: f32,
        seed: u64,
    ) -> Self {
        let model = &CHASSIS[chassis];
        let wheels = model.wheels();
        let mut hull = body_box(model, &model.wheels);
        let top = hull.center.y + hull.half.y;
        hull.center.y = top / 2.0;
        hull.half.y = top / 2.0;
        let mass = model.mass + DRIVETRAIN_MASS + 2.0 * (model.unsprung(0) + model.unsprung(1));
        let size = Vec3::new(model.body.width, 1.0, model.body.length);
        let mut body = RigidBody::new(
            Vec3::ZERO,
            Quat::IDENTITY,
            mass,
            RigidBody::box_inertia(mass, size),
        );
        body.center = Vec3::new(0.0, model.cg_height, 0.0);
        let mut driver = Driver {
            route: VecDeque::from([leg]),
            start: s,
            along: 0.0,
            rear: Vec2::ZERO,
            speed: 0.0,
            keen,
            rng: Rng(seed | 1),
        };
        driver.plan(network);
        let mut path = Path::default();
        path.build(network, &driver.route, driver.start);
        // Starting at a little under the lowest limit just ahead.
        let lowest = (path.at.iter().zip(&path.limits))
            .take_while(|&(&at, _)| at < 150.0)
            .map(|(_, &limit)| limit)
            .fold(f32::INFINITY, f32::min);
        driver.speed = 0.8 * keen * lowest;
        let (front, direction) = path.point(0.0);
        driver.rear = front - direction * wheels.wheelbase;
        let mut car = Self {
            body,
            hull,
            wheels,
            driver: None,
            rolled: 0.0,
            steer: 0.0,
            still: 0.0,
        };
        car.place(terrain, front, driver.rear, driver.speed, 0.0);
        car.driver = Some(driver);
        car
    }

    /// Sets the body on the ground under its axles, moving at `speed` and turning at
    /// `yaw_rate`.
    fn place(&mut self, terrain: &Terrain, front: Vec2, rear: Vec2, speed: f32, yaw_rate: f32) {
        let forward = (front - rear).normalize_or(Vec2::NEG_Y);
        let right = Vec2::new(-forward.y, forward.x);
        let middle = (front + rear) / 2.0;
        let height = |p: Vec2| terrain.sample(p.x, p.y).0;
        let track = (self.wheels.track[0] + self.wheels.track[1]) / 2.0;
        let (hf, hr) = (height(front), height(rear));
        let (hl, hrt) = (
            height(middle - right * track / 2.0),
            height(middle + right * track / 2.0),
        );
        let ahead = Vec3::new(
            forward.x * self.wheels.wheelbase,
            hf - hr,
            forward.y * self.wheels.wheelbase,
        )
        .normalize();
        let side = Vec3::new(right.x * track, hrt - hl, right.y * track).normalize();
        let up = side.cross(ahead).normalize();
        let side = ahead.cross(up);
        self.body.rotation = Quat::from_mat3(&Mat3::from_cols(side, up, -ahead));
        self.body.position = Vec3::new(middle.x, (hf + hr) / 2.0, middle.y);
        self.body.velocity = ahead * speed;
        self.body.angular_velocity = up * yaw_rate;
    }

    /// The box where the car is now.
    fn placed_hull(&self) -> Obb {
        self.hull.placed(self.body.position, self.body.rotation)
    }
}

/// What's on the road, for drivers to keep their distance to: where, how fast, and
/// how long and wide (half, m).
struct Vehicle {
    at: Vec2,
    velocity: Vec2,
    /// Which way it points (unit).
    forward: Vec2,
    half_length: f32,
    half_width: f32,
}

impl Vehicle {
    fn of(body: &RigidBody, hull: &Obb) -> Self {
        Self {
            at: body.position.xz(),
            velocity: body.velocity.xz(),
            forward: (body.rotation * Vec3::NEG_Z).xz().normalize_or(Vec2::NEG_Y),
            half_length: hull.half.z,
            half_width: hull.half.x,
        }
    }
}

/// Moves every traffic car on: drivers along their routes, crashed cars as loose bodies;
/// then the cars that touch each other or the player's crash.
fn step(
    time: Res<Time>,
    terrain: Res<Terrain>,
    network: Res<Network>,
    mut traffic: Query<&mut TrafficCar>,
    mut cars: Query<&mut Car>,
) {
    let dt = time.delta_secs();
    let vehicles: Vec<Vehicle> = (traffic.iter().map(|car| Vehicle::of(&car.body, &car.hull)))
        .chain(
            cars.iter()
                .map(|car| Vehicle::of(&car.body, &car.setup.hull)),
        )
        .collect();
    let mut path = Path::default();
    let mut contacts = Vec::new();
    for (i, mut car) in traffic.iter_mut().enumerate() {
        if car.driver.is_some() {
            drive(&mut car, i, &vehicles, &network, &terrain, &mut path, dt);
        } else {
            tumble(&mut car, &terrain, &mut contacts, dt);
        }
    }

    // Knocks between traffic cars, and with the player's.
    let mut pairs = traffic.iter_combinations_mut();
    while let Some([mut a, mut b]) = pairs.fetch_next() {
        if let Some(contact) = touch(&a.placed_hull(), &b.placed_hull()) {
            crash(&mut a);
            crash(&mut b);
            collision::resolve(&mut a.body, Some(&mut b.body), &[contact], OBSTACLE, dt);
        }
    }
    for mut player in &mut cars {
        let hull = player
            .setup
            .hull
            .placed(player.body.position, player.body.rotation);
        for mut car in &mut traffic {
            if let Some(contact) = touch(&hull, &car.placed_hull()) {
                crash(&mut car);
                let impulse = collision::resolve(
                    &mut player.body,
                    Some(&mut car.body),
                    &[contact],
                    OBSTACLE,
                    dt,
                );
                player.knocks += impulse / player.body.mass;
            }
        }
    }
}

/// Where two boxes overlap, if their spheres even touch.
fn touch(a: &Obb, b: &Obb) -> Option<Contact> {
    let reach = a.radius() + b.radius();
    if a.center.distance_squared(b.center) > reach * reach {
        return None;
    }
    a.contact(b)
}

/// The driver's done: the car is a loose body from now on (and awake).
fn crash(car: &mut TrafficCar) {
    car.driver = None;
    car.still = 0.0;
}

/// A driver's step: plan, choose a speed, move along the path.
fn drive(
    car: &mut TrafficCar,
    me: usize,
    vehicles: &[Vehicle],
    network: &Network,
    terrain: &Terrain,
    path: &mut Path,
    dt: f32,
) {
    let wheelbase = car.wheels.wheelbase;
    let Some(driver) = car.driver.as_mut() else {
        return;
    };
    driver.plan(network);
    path.build(network, &driver.route, driver.start);
    // Past the corner into the next leg, that leg is the one it's on.
    while let Some(seam) = path.seam.filter(|&seam| driver.along >= seam) {
        let leg = driver.route.pop_front().unwrap();
        driver.start = corner(network, leg, driver.route[0]);
        driver.along -= seam;
        driver.plan(network);
        path.build(network, &driver.route, driver.start);
    }

    // The fastest it can go here and still slow down in time for every bend and limit
    // ahead.
    let along = driver.along;
    let first = path.at.partition_point(|&a| a <= along);
    let mut allowed = driver.keen * path.limits[first.saturating_sub(1)];
    for k in first..path.points.len() {
        let d = path.at[k] - along;
        if d > LOOKAHEAD {
            break;
        }
        let (back, _) = path.point(path.at[k] - CHORD);
        let (ahead, _) = path.point(path.at[k] + CHORD);
        let p = path.points[k];
        let turn = (p - back).angle_to(ahead - p).abs();
        let bend = (CORNERING * CHORD / turn.max(1e-3)).sqrt();
        let v = (driver.keen * path.limits[k]).min(bend);
        allowed = allowed.min((v * v + 2.0 * COMFORT_BRAKING * d).sqrt());
    }

    // The nearest thing in the lane ahead: its gap and its speed along the lane.
    let mine = &vehicles[me];
    let mut leader: Option<(f32, f32)> = None;
    for (j, other) in vehicles.iter().enumerate() {
        if j == me || other.at.distance_squared(mine.at) > (FOLLOW_RANGE + 10.0).powi(2) {
            continue;
        }
        for k in first.saturating_sub(1)..path.points.len().saturating_sub(1) {
            let start = path.at[k] - along;
            if start > FOLLOW_RANGE {
                break;
            }
            let (a, b) = (path.points[k], path.points[k + 1]);
            let d = b - a;
            let t = ((other.at - a).dot(d) / d.length_squared().max(1e-6)).clamp(0.0, 1.0);
            // How far it reaches across the lane; a car coming the other way only counts
            // once it's in this lane.
            let direction = d.normalize_or_zero();
            let across = other.half_length * other.forward.perp_dot(direction).abs()
                + other.half_width * other.forward.dot(direction).abs();
            let width = if other.velocity.dot(direction) < -1.0 {
                ONCOMING
            } else {
                mine.half_width + across + IN_LANE
            };
            if other.at.distance_squared(a + d * t) > width * width {
                continue;
            }
            // From the front bumper (the front axle is half a wheelbase ahead of the
            // middle) to the other's back.
            let to = start + t * (path.at[k + 1] - path.at[k]);
            let gap = to - other.half_length - (mine.half_length - wheelbase / 2.0);
            if to > -0.5 * mine.half_length && leader.is_none_or(|(g, _)| gap < g) {
                leader = Some((gap, other.velocity.dot(direction).max(0.0)));
            }
            break;
        }
    }

    // The intelligent driver model.
    let v = driver.speed;
    let mut acceleration = ACCELERATION * (1.0 - (v / allowed.max(0.1)).powi(4));
    // Over what's allowed, it brakes enough to get back to it quickly.
    if v > allowed {
        acceleration = acceleration.min((allowed - v) / TRACKING);
    }
    if let Some((gap, speed)) = leader {
        let wanted = STANDSTILL_GAP
            + (v * TIME_GAP + v * (v - speed) / (2.0 * (ACCELERATION * COMFORT_BRAKING).sqrt()))
                .max(0.0);
        acceleration -= ACCELERATION * (wanted / gap.max(0.1)).powi(2);
    }
    driver.speed = (v + acceleration.max(-HARDEST_BRAKING) * dt).max(0.0);
    let (front, direction) = path.point(driver.along);
    let old = (front - driver.rear).normalize_or(direction);
    driver.along += driver.speed * dt;

    // The front axle on the path, the rear dragged after it.
    let (front, direction) = path.point(driver.along);
    let rear = front - (front - driver.rear).normalize_or(-direction) * wheelbase;
    let heading = (front - rear).normalize_or(direction);
    driver.rear = rear;
    let yaw_rate = -old.angle_to(heading) / dt.max(1e-6);
    let steer = heading.angle_to(direction);
    let speed = driver.speed;
    let rear = driver.rear;
    car.steer = -steer;
    car.rolled += speed * dt;
    car.place(terrain, front, rear, speed, yaw_rate);
}

/// A crashed car: a loose body on its box, against the ground, houses and trees.
fn tumble(car: &mut TrafficCar, terrain: &Terrain, contacts: &mut Vec<Contact>, dt: f32) {
    let body = &mut car.body;
    if body.velocity.length() < STILL && body.angular_velocity.length() < STILL {
        car.still += dt;
        if car.still > SETTLE_TIME {
            body.velocity = Vec3::ZERO;
            body.angular_velocity = Vec3::ZERO;
            return;
        }
    } else {
        car.still = 0.0;
    }
    body.add_force_at_center(GRAVITY * body.mass);
    body.integrate_velocity(dt);
    let hull = car.hull.placed(body.position, body.rotation);
    contacts.clear();
    collision::ground_contacts(&hull, |x, z| terrain.sample(x, z), contacts);
    collision::resolve(body, None, contacts, GROUND, dt);
    contacts.clear();
    terrain.obstacles.contacts(&hull, contacts);
    collision::resolve(body, None, contacts, OBSTACLE, dt);
    body.integrate_position(dt);
    let forward = body.rotation * Vec3::NEG_Z;
    car.rolled += body.velocity.dot(forward) * dt;
}

/// Puts the cars where they are and turns and steers their wheels.
fn pose(
    mut cars: Query<(&TrafficCar, &mut Transform), Without<WheelPart>>,
    mut parts: Query<(&WheelPart, &ChildOf, &mut Transform)>,
) {
    for (car, mut transform) in &mut cars {
        transform.translation = car.body.position;
        transform.rotation = car.body.rotation;
    }
    for (part, child_of, mut transform) in &mut parts {
        let Ok((car, _)) = cars.get(child_of.parent()) else {
            continue;
        };
        let w = &car.wheels;
        let axle = part.corner / 2;
        let side = if part.corner % 2 == 0 { -1.0 } else { 1.0 };
        let radius = w.radius[axle];
        let width = w.width[axle];
        let z = if axle == 0 { -0.5 } else { 0.5 } * w.wheelbase;
        let steer = if axle == 0 { car.steer } else { 0.0 };
        let rotation = Quat::from_rotation_y(steer)
            * Quat::from_rotation_x(-car.rolled / radius)
            * Quat::from_rotation_z(FRAC_PI_2);
        let (rotation, scale) = match part.kind {
            WheelKind::Tire => (rotation, Vec3::new(radius, width, radius)),
            WheelKind::Rim => (
                rotation,
                Vec3::new(RIM * radius, width + 0.01, RIM * radius),
            ),
            WheelKind::Spoke => (
                rotation,
                Vec3::new(1.9 * RIM * radius, width + 0.02, 0.22 * radius),
            ),
        };
        *transform = Transform {
            translation: Vec3::new(side * w.track[axle] / 2.0, radius, z),
            rotation,
            scale,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drivers on the map's roads for a minute: they keep moving, stay near the limits
    /// and seldom run into each other (only at junctions).
    #[test]
    fn drivers_follow_the_roads() {
        let terrain = Terrain::baked();
        let network = Network::new(&terrain);
        let spawner = Spawner::new(&network);
        let mut rng = Rng(12345);
        let mut cars: Vec<TrafficCar> = Vec::new();
        // Thirty cars on the roads near the spawn point.
        let center = terrain.road_spawns(1, 0.0)[0].0;
        let mut spots: Vec<_> = spawner
            .spots
            .iter()
            .filter(|spot| spot.2.distance(center) < 600.0)
            .collect();
        for i in (1..spots.len()).rev() {
            spots.swap(i, (rng.next() * i as f32) as usize);
        }
        for &&(leg, s, p) in &spots {
            if cars.len() == 30 {
                break;
            }
            if cars
                .iter()
                .all(|car| car.body.position.xz().distance(p) > SPAWN_GAP)
            {
                let chassis = rng.pick(COMMON.iter().copied());
                let keen = rng.range(KEEN);
                cars.push(TrafficCar::new(
                    &network, &terrain, chassis, leg, s, keen, rng.0,
                ));
            }
        }
        let dt = 1.0 / 120.0;
        let mut path = Path::default();
        let mut contacts = Vec::new();
        let mut travelled = 0.0;
        let mut too_fast = 0;
        let start = std::time::Instant::now();
        for _ in 0..60 * 120 {
            let vehicles: Vec<Vehicle> = cars
                .iter()
                .map(|car| Vehicle::of(&car.body, &car.hull))
                .collect();
            for (i, car) in cars.iter_mut().enumerate() {
                if car.driver.is_some() {
                    drive(car, i, &vehicles, &network, &terrain, &mut path, dt);
                    let driver = car.driver.as_ref().unwrap();
                    travelled += driver.speed * dt;
                    let limit = network.limit(driver.route[0], driver.start + driver.along);
                    if driver.speed > KEEN.1 * limit * 1.05 + 0.5 {
                        too_fast += 1;
                    }
                } else {
                    tumble(car, &terrain, &mut contacts, dt);
                }
            }
            for i in 0..cars.len() {
                for j in i + 1..cars.len() {
                    let (a, b) = cars.split_at_mut(j);
                    let (a, b) = (&mut a[i], &mut b[0]);
                    if let Some(contact) = touch(&a.placed_hull(), &b.placed_hull()) {
                        crash(a);
                        crash(b);
                        collision::resolve(
                            &mut a.body,
                            Some(&mut b.body),
                            &[contact],
                            OBSTACLE,
                            dt,
                        );
                    }
                }
            }
        }
        let crashed = cars.iter().filter(|car| car.driver.is_none()).count();
        let average = travelled / (60.0 * cars.len() as f32);
        println!(
            "{} cars, {crashed} crashed, average {:.0} km/h, {too_fast} steps too fast, {:?} per step",
            cars.len(),
            average * 3.6,
            start.elapsed() / (60 * 120)
        );
        assert_eq!(cars.len(), 30);
        assert!(average > 6.0, "average {average} m/s");
        assert_eq!(too_fast, 0);
        assert!(crashed <= 8, "{crashed} crashed");
        // Each driver's front axle is on its lane.
        for car in cars.iter().filter(|car| car.driver.is_some()) {
            let driver = car.driver.as_ref().unwrap();
            path.build(&network, &driver.route, driver.start);
            let (front, _) = path.point(driver.along);
            let forward = car.body.rotation * Vec3::NEG_Z;
            let axle =
                car.body.position.xz() + forward.xz().normalize() * car.wheels.wheelbase / 2.0;
            assert!(axle.distance(front) < 0.3, "{axle} {front}");
        }
    }
}
