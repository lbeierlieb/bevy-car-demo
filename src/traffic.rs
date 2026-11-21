//! Traffic: cars driving the roads around the players' cars.
//!
//! They aren't simulated like the player's car. A driver follows a route through the road
//! network (`network.rs`), picking a way at random at each junction; the front axle runs
//! along the lane, the rear axle is dragged after it, and the body sits on the ground
//! under the wheels. The speed follows the limits, slowing for bends and junctions
//! ahead, and keeps a distance to whatever is in the lane ahead (the intelligent driver
//! model). Any knock against another car (or the player's) ends the drive: the car is a
//! loose rigid body on its box from then on, sliding and tumbling to a stop. Cars come
//! and go out of sight around the players: every simulated `Car` and every `Anchor` (on
//! the server, the players driving elsewhere). `TrafficPlugin` is the simulation alone
//! (the server has it). Each car also has the `Looks` and `Motion` of its chassis as it
//! leaves the factory (`show`), so it's drawn (and sent to players) like any other car.

pub mod network;

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::audio::EngineRun;
use crate::body::Wheels;
use crate::car::{Car, CarSetup, body_box};
use crate::chassis::{CHASSIS, PAINTS};
use crate::looks::{Looks, Motion, WheelMotion};
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
/// What a traffic car's engine sounds like: idling standing, cruising at a steady rpm
/// and load once moving (no gears), blended in over the first few m/s.
const IDLE_RPM: f32 = 850.0;
const CRUISE_RPM: f32 = 2000.0;
const CRUISE_LOAD: f32 = 0.2;
const CRUISING_FROM: f32 = 3.0;

/// How many traffic cars drive around each player (the panel's traffic density).
#[derive(Resource)]
pub struct Traffic {
    pub cars: usize,
}

impl Default for Traffic {
    fn default() -> Self {
        Self { cars: DEFAULT_CARS }
    }
}

/// The traffic's simulation: drivers, crashes, cars coming and going around the players.
pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Traffic>()
            .add_systems(Startup, |mut commands: Commands, terrain: Res<Terrain>| {
                let network = Network::new(&terrain);
                commands.insert_resource(Spawner::new(&network));
                commands.insert_resource(network);
            })
            .init_resource::<Source>()
            .init_resource::<FactoryLooks>()
            .init_resource::<CarNumbers>()
            .add_systems(Update, populate.run_if(is_own))
            .add_systems(Update, show.after(populate).before(crate::looks::show_cars))
            .add_systems(FixedUpdate, step.after(crate::sim::simulate).run_if(is_own));
    }
}

/// Where the traffic comes from: simulated here (alone, and on the server), or from the
/// server (a player on one): then there is none here, the server's cars come like any
/// other car the server sends (`net::client::RemoteCar`).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Own,
    Server,
}

fn is_own(source: Res<Source>) -> bool {
    *source == Source::Own
}

/// A car driven somewhere else (on the server, a player's): traffic keeps its distance
/// and crashes into it, but doesn't move it. Its body is where it is now.
#[derive(Component, Clone)]
pub struct Visitor {
    pub body: RigidBody,
    pub hull: Obb,
}

/// Numbers for the cars: one counter for the traffic and (on the server) the players'
/// cars, so a car's number doesn't tell which it is.
#[derive(Resource)]
pub struct CarNumbers(u32);

impl Default for CarNumbers {
    fn default() -> Self {
        Self(1)
    }
}

impl CarNumbers {
    /// A number no car had yet.
    pub fn take(&mut self) -> u32 {
        let number = self.0;
        self.0 = self.0.wrapping_add(1);
        number
    }
}

/// What each chassis looks like as it leaves the factory (with its base engine), made
/// when first needed.
#[derive(Resource, Default)]
pub struct FactoryLooks(Vec<Option<Looks>>);

impl FactoryLooks {
    /// The factory's `chassis` (in `CHASSIS`) in `paint` (in `PAINTS`).
    pub fn get(&mut self, chassis: usize, paint: usize) -> Looks {
        if self.0.is_empty() {
            self.0 = vec![None; CHASSIS.len()];
        }
        let mut looks = self.0[chassis]
            .get_or_insert_with(|| Looks::of(&CarSetup::factory(chassis, 0)))
            .clone();
        looks.build.paint = paint;
        looks
    }
}

/// Gives every traffic car the looks of its chassis, and shows how it moves: where its
/// centre of mass is (its `Transform`, as for any car with `Looks`), its wheels rolling
/// and steering on a suspension at rest, and its engine idling.
fn show(
    mut commands: Commands,
    mut factory: ResMut<FactoryLooks>,
    mut new: Query<(Entity, &TrafficCar, &mut Transform), Without<Looks>>,
    mut cars: Query<(&TrafficCar, &Looks, &mut Transform, &mut Motion)>,
) {
    for (entity, car, mut transform) in &mut new {
        let looks = factory.get(car.chassis, car.paint);
        *transform = car.transform(&looks);
        commands.entity(entity).insert((car.motion(&looks), looks));
    }
    for (car, looks, mut transform, mut motion) in &mut cars {
        *transform = car.transform(looks);
        *motion = car.motion(looks);
    }
}

/// Someone traffic drives around, besides every simulated `Car`: where they are is the
/// entity's `Transform`.
#[derive(Component)]
pub struct Anchor;

/// A traffic car.
#[derive(Component)]
pub struct TrafficCar {
    /// Its number (from `CarNumbers`), for the server to tell players which is which.
    pub id: u32,
    /// What it looks like: its chassis (in `CHASSIS`) and paint (in `PAINTS`).
    pub chassis: usize,
    pub paint: usize,
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
    /// The way ahead along `route` from `start`, built again only when they change.
    path: Path,
}

/// The way ahead of a driver, along the lanes of its route with the corners at the
/// junctions rounded: points, their distance from the start, and the limit at each.
#[derive(Default)]
struct Path {
    points: Vec<Vec2>,
    at: Vec<f32>,
    limits: Vec<f32>,
    /// The speed the bend at each point allows (m/s).
    bends: Vec<f32>,
    /// Where the second leg begins (past the corner into it), if there's one.
    seam: Option<f32>,
}

impl Path {
    fn clear(&mut self) {
        self.points.clear();
        self.at.clear();
        self.limits.clear();
        self.bends.clear();
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
        self.bends = (0..self.points.len())
            .map(|k| {
                let (back, _) = self.point(self.at[k] - CHORD);
                let (ahead, _) = self.point(self.at[k] + CHORD);
                let p = self.points[k];
                let turn = (p - back).angle_to(ahead - p).abs();
                (CORNERING * CHORD / turn.max(1e-3)).sqrt()
            })
            .collect();
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
    /// at random, wide roads more likely, turning round at dead ends. Whether it planned
    /// more.
    fn plan(&mut self, network: &Network) -> bool {
        let mut planned = false;
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
            planned = true;
        }
        planned
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
        }
    }
}

/// Whom traffic drives around.
type Player = Or<(With<Car>, With<Anchor>)>;

/// Keeps the number of cars around each player at what's asked for: new ones appear out
/// of everyone's sight on the roads around the player with the fewest, ones left far
/// behind by everyone go (and when there are too many, the one farthest from anyone,
/// among those only near players with too many). Players near each other share the cars
/// around them, so the traffic is as dense where many drive as where one does.
#[allow(clippy::too_many_arguments)]
fn populate(
    mut commands: Commands,
    traffic: Res<Traffic>,
    network: Res<Network>,
    terrain: Res<Terrain>,
    mut spawner: ResMut<Spawner>,
    mut numbers: ResMut<CarNumbers>,
    players: Query<&Transform, Player>,
    others: Query<(Entity, &TrafficCar)>,
) {
    let centres: Vec<Vec2> = players.iter().map(|t| t.translation.xz()).collect();
    if centres.is_empty() {
        return;
    }
    let spawner = &mut *spawner;
    let nearest = |p: Vec2| (centres.iter().map(|c| c.distance(p))).fold(f32::INFINITY, f32::min);
    // The cars that stay, and how many are around each player.
    let mut kept = Vec::new();
    let mut around = vec![0; centres.len()];
    for (entity, car) in &others {
        let p = car.body.position.xz();
        let reach = if car.driver.is_some() {
            DESPAWN
        } else {
            DESPAWN_CRASHED
        };
        if nearest(p) > reach {
            commands.entity(entity).despawn();
            continue;
        }
        for (count, c) in around.iter_mut().zip(&centres) {
            *count += usize::from(c.distance(p) <= DESPAWN);
        }
        kept.push((entity, p));
    }
    // Too many: the car farthest from anyone, whose players all have too many.
    let surplus = kept
        .iter()
        .filter(|&&(_, p)| {
            (around.iter().zip(&centres))
                .all(|(&count, c)| c.distance(p) > DESPAWN || count > traffic.cars)
        })
        .max_by(|a, b| nearest(a.1).total_cmp(&nearest(b.1)));
    if around.iter().any(|&count| count > traffic.cars)
        && let Some(&(entity, _)) = surplus
    {
        commands.entity(entity).despawn();
        return;
    }
    // Too few: one more around the player who has the fewest.
    let Some((lacking, _)) = (around.iter().enumerate())
        .filter(|&(_, &count)| count < traffic.cars)
        .min_by_key(|&(_, &count)| count)
    else {
        spawner.started = true;
        return;
    };
    let center = centres[lacking];

    // A spot on the roads in the ring around that player, out of everyone's sight and
    // clear of every other car.
    let near = if spawner.started {
        SPAWN_NEAR
    } else {
        SPAWN_FIRST
    };
    let taken: Vec<Vec2> = (kept.iter().map(|&(_, p)| p))
        .chain(centres.iter().copied())
        .collect();
    let free: Vec<usize> = (0..spawner.spots.len())
        .filter(|&i| {
            let p = spawner.spots[i].2;
            p.distance(center) < SPAWN_FAR
                && nearest(p) > near
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
    let mut car = TrafficCar::new(&network, &terrain, chassis, paint, leg, s, keen, seed);
    car.id = numbers.take();
    commands.spawn((
        Transform::from_translation(car.body.position).with_rotation(car.body.rotation),
        car,
    ));
}

impl TrafficCar {
    /// A car of `chassis` (in `CHASSIS`) and `paint` with its front axle `s` along `leg`'s
    /// lane, driving off at a little under the limit.
    #[allow(clippy::too_many_arguments)]
    fn new(
        network: &Network,
        terrain: &Terrain,
        chassis: usize,
        paint: usize,
        leg: Leg,
        s: f32,
        keen: f32,
        seed: u64,
    ) -> Self {
        let mut car = Self::parked(chassis, paint);
        let wheels = car.wheels;
        let mut driver = Driver {
            route: VecDeque::from([leg]),
            start: s,
            along: 0.0,
            rear: Vec2::ZERO,
            speed: 0.0,
            keen,
            rng: Rng(seed | 1),
            path: Path::default(),
        };
        driver.plan(network);
        let path = &mut driver.path;
        path.build(network, &driver.route, driver.start);
        // Starting at a little under the lowest limit just ahead.
        let lowest = (path.at.iter().zip(&path.limits))
            .take_while(|&(&at, _)| at < 150.0)
            .map(|(_, &limit)| limit)
            .fold(f32::INFINITY, f32::min);
        driver.speed = 0.8 * keen * lowest;
        let (front, direction) = path.point(0.0);
        driver.rear = front - direction * wheels.wheelbase;
        // On a bridge's deck, or under any.
        car.body.position.y = if network.edges[leg.edge as usize].bridge {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
        car.place(terrain, front, driver.rear, driver.speed, 0.0);
        car.driver = Some(driver);
        car
    }

    /// A car of `chassis` (in `CHASSIS`) and `paint` without a driver, at the origin.
    fn parked(chassis: usize, paint: usize) -> Self {
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
        Self {
            id: 0,
            chassis,
            paint,
            body,
            hull,
            wheels,
            driver: None,
            rolled: 0.0,
            steer: 0.0,
            still: 0.0,
        }
    }

    /// Where its centre of mass is, for a car that `looks` so (its `Transform`).
    pub fn transform(&self, looks: &Looks) -> Transform {
        let rotation = self.body.rotation;
        Transform::from_translation(self.body.position - rotation * looks.body_offset)
            .with_rotation(rotation)
    }

    /// What moves on it, for a car that `looks` so: the wheels roll and the front ones
    /// steer, on a suspension at rest; the engine idles standing and cruises moving.
    pub fn motion(&self, looks: &Looks) -> Motion {
        let moving = (self.body.velocity.length() / CRUISING_FROM).min(1.0);
        Motion {
            wheels: std::array::from_fn(|corner| {
                let axle = &looks.axles[corner / 2];
                WheelMotion {
                    theta: axle.geometry.design_angle(),
                    steer: if corner < 2 { self.steer } else { 0.0 },
                    spin: self.rolled / axle.radius,
                }
            }),
            engine: EngineRun {
                running: true,
                rpm: IDLE_RPM + (CRUISE_RPM - IDLE_RPM) * moving,
                load: CRUISE_LOAD * moving,
                turbo_speed: 0.0,
            },
        }
    }

    /// How fast its centre of mass moves, for a car that `looks` so.
    pub fn center_velocity(&self, looks: &Looks) -> Vec3 {
        let body = &self.body;
        let arm = -(body.rotation * looks.body_offset);
        body.velocity + body.angular_velocity.cross(arm)
    }

    /// Sets the body on the ground under its axles, moving at `speed` and turning at
    /// `yaw_rate`.
    fn place(&mut self, terrain: &Terrain, front: Vec2, rear: Vec2, speed: f32, yaw_rate: f32) {
        let forward = (front - rear).normalize_or(Vec2::NEG_Y);
        let right = Vec2::new(-forward.y, forward.x);
        let middle = (front + rear) / 2.0;
        // On the level the car is on (from the last step, or where it was spawned).
        let level = self.body.position.y;
        let height = |p: Vec2| terrain.ground(p.x, p.y, level).height;
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
    visitors: Query<&Visitor>,
) {
    let dt = time.delta_secs();
    let vehicles: Vec<Vehicle> = (traffic.iter().map(|car| Vehicle::of(&car.body, &car.hull)))
        .chain(
            cars.iter()
                .map(|car| Vehicle::of(&car.body, &car.setup.hull)),
        )
        .chain(visitors.iter().map(|v| Vehicle::of(&v.body, &v.hull)))
        .collect();
    let mut contacts = Vec::new();
    for (i, mut car) in traffic.iter_mut().enumerate() {
        if car.driver.is_some() {
            drive(&mut car, i, &vehicles, &network, &terrain, dt);
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
    // Cars driven elsewhere: only the traffic car is moved (the other is pushed off on
    // its own side).
    for visitor in &visitors {
        let hull = visitor
            .hull
            .placed(visitor.body.position, visitor.body.rotation);
        for mut car in &mut traffic {
            if let Some(contact) = touch(&car.placed_hull(), &hull) {
                crash(&mut car);
                let mut other = visitor.body.clone();
                collision::resolve(&mut car.body, Some(&mut other), &[contact], OBSTACLE, dt);
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
    dt: f32,
) {
    let wheelbase = car.wheels.wheelbase;
    let Some(driver) = car.driver.as_mut() else {
        return;
    };
    let mut path = std::mem::take(&mut driver.path);
    if driver.plan(network) || path.points.is_empty() {
        path.build(network, &driver.route, driver.start);
    }
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
        let v = (driver.keen * path.limits[k]).min(path.bends[k]);
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
    driver.path = path;
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
    collision::ground_contacts(&hull, |p| terrain.ground_under(p), contacts);
    collision::resolve(body, None, contacts, GROUND, dt);
    contacts.clear();
    terrain.obstacles.contacts(&hull, contacts);
    collision::resolve(body, None, contacts, OBSTACLE, dt);
    body.integrate_position(dt);
    let forward = body.rotation * Vec3::NEG_Z;
    car.rolled += body.velocity.dot(forward) * dt;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Players near each other share the traffic around them; one far away has their
    /// own.
    #[test]
    fn traffic_is_shared_by_players_near_each_other() {
        let terrain = Terrain::baked();
        let (spawn, _) = terrain.road_spawns(1, 0.0)[0];
        // Two players 80 m apart, a third on the roads 2 km or more away.
        let network = Network::new(&terrain);
        let far = Spawner::new(&network)
            .spots
            .iter()
            .map(|spot| spot.2)
            .find(|p| p.distance(spawn) > 2000.0 && terrain.inside(p.x, p.y))
            .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(terrain)
            .add_plugins(TrafficPlugin);
        let at = |p: Vec2| Transform::from_xyz(p.x, 0.0, p.y);
        for p in [spawn, spawn + Vec2::new(80.0, 0.0), far] {
            app.world_mut().spawn((at(p), Anchor));
        }
        // One car comes or goes per frame.
        for _ in 0..200 {
            app.update();
        }
        let cars: Vec<Vec2> = (app.world_mut())
            .query::<&TrafficCar>()
            .iter(app.world())
            .map(|car| car.body.position.xz())
            .collect();
        let around = |c: Vec2| cars.iter().filter(|p| p.distance(c) <= DESPAWN).count();
        println!(
            "{} cars: {} around the two, {} around the far one",
            cars.len(),
            around(spawn),
            around(far)
        );
        assert!(around(spawn) >= DEFAULT_CARS && around(far) >= DEFAULT_CARS);
        assert!(cars.len() < 3 * DEFAULT_CARS, "the two near players share");
    }

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
                    &network, &terrain, chassis, 0, leg, s, keen, rng.0,
                ));
            }
        }
        let dt = 1.0 / 120.0;
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
                    drive(car, i, &vehicles, &network, &terrain, dt);
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
            let mut path = Path::default();
            path.build(&network, &driver.route, driver.start);
            let (front, _) = path.point(driver.along);
            let forward = car.body.rotation * Vec3::NEG_Z;
            let axle =
                car.body.position.xz() + forward.xz().normalize() * car.wheels.wheelbase / 2.0;
            assert!(axle.distance(front) < 0.3, "{axle} {front}");
        }
    }
}
