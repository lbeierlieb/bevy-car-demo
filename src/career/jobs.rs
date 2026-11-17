//! Jobs: delivering a pizza to a house (in the player's car, against the clock) or a car
//! from the dealer (in that car, which the player then leaves there). The destination has
//! a marker, is on the map, and an arrow at the top points to it; stopping on the marker
//! delivers. Knocks on the way cost: the pizza's tip, or the dealer's trust and pay.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Shape, Stroke, pos2, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hud::{News, Status};
use super::places::{MARKER_RADIUS, STOPPED, kerb, spawn_marker};
use super::{Career, PLAYER, Places, Scene, euros};
use crate::buildings::Footprint;
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Car};
use crate::map::MapMark;
use crate::map::MapMarks;
use crate::terrain::Terrain;

/// A knock that changes the speed by more than this is a hit (m/s); hits closer together
/// than this count as one (s).
const HIT: f32 = 1.2;
const HIT_GAP: f32 = 1.0;
/// Pizzas go to houses this far from the pizzeria (m, as the crow flies).
const PIZZA_RANGE: (f32, f32) = (400.0, 3_000.0);
/// A pizza pays this to start with and per kilometre (€), with a tip on top for one on
/// time and without a knock; the time it has is a minute and this many seconds a
/// kilometre (roads wind).
const PIZZA_BASE: f64 = 10.0;
const PIZZA_PER_KM: f64 = 9.0;
const PIZZA_TIP: f64 = 0.3;
const PIZZA_SECONDS_PER_KM: f32 = 130.0;
/// Destinations are drawn in this colour.
const DESTINATION: [u8; 3] = [255, 255, 255];

pub struct JobsPlugin;

impl Plugin for JobsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Jobs>()
            .add_systems(
                Update,
                (
                    seed,
                    (watch, deliver).chain().run_if(in_state(Scene::Driving)),
                ),
            )
            .add_systems(OnExit(Scene::Driving), give_up)
            .add_systems(
                EguiPrimaryContextPass,
                (arrow, job_window).run_if(in_state(Scene::Driving)),
            );
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum JobKind {
    Pizza,
}

/// A job on offer or under way.
pub struct Job {
    pub kind: JobKind,
    /// Where to stop (on the road at the house), as the crow flies from where it starts.
    pub destination: Vec2,
    pub distance: f32,
    /// Full pay (€), and the time there is (s; pizzas only).
    pub pay: f64,
    pub time: Option<f32>,
    /// Under way: since when (s), how often it hit something, and the destination's marker.
    pub elapsed: f32,
    pub hits: u32,
    pub marker: Option<Entity>,
}

impl Job {
    /// What it pays now: less when late, less for each hit (a pizza loses its tip).
    pub fn earned(&self) -> f64 {
        match self.kind {
            JobKind::Pizza => {
                let limit = self.time.unwrap_or(f32::INFINITY);
                let late = ((self.elapsed - limit) / limit).max(0.0);
                let on_time = (1.0 - f64::from(late)).max(0.3);
                let tip = if self.hits == 0 && late == 0.0 {
                    PIZZA_TIP
                } else {
                    0.0
                };
                self.pay * (on_time + tip)
            }
        }
    }
}

/// The job under way, the one on offer at the place the player is at, and the dice.
#[derive(Resource, Default)]
pub struct Jobs {
    pub current: Option<Job>,
    pub offer: Option<Job>,
    rng: Option<Rng>,
    /// Since the last hit (s).
    since_hit: f32,
}

impl Jobs {
    pub fn rng(&mut self) -> &mut Rng {
        self.rng.get_or_insert(Rng(0x9e37_79b9_7f4a_7c15))
    }

    /// A pizza from `from` to a house somewhere around it.
    pub fn pizza(&mut self, terrain: &Terrain, from: Vec2) -> Option<Job> {
        let destination = pick_house(terrain, self.rng(), from, PIZZA_RANGE)?;
        let distance = destination.distance(from);
        let km = f64::from(distance) / 1000.0;
        Some(Job {
            kind: JobKind::Pizza,
            destination,
            distance,
            pay: PIZZA_BASE + PIZZA_PER_KM * km,
            time: Some(60.0 + PIZZA_SECONDS_PER_KM * distance / 1000.0),
            elapsed: 0.0,
            hits: 0,
            marker: None,
        })
    }
}

/// xorshift64*: dice for jobs and cars on offer.
pub struct Rng(u64);

impl Rng {
    /// From 0 up to 1.
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n.saturating_sub(1))
    }
}

/// The dice start from when the game first asks, so every run differs.
fn seed(time: Res<Time<Real>>, mut jobs: ResMut<Jobs>) {
    if jobs.rng.is_none() && time.elapsed_secs() > 0.0 {
        let nanos = time.elapsed().as_nanos() as u64;
        jobs.rng = Some(Rng(nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1));
    }
}

/// A house `range` from `from` (as the crow flies) with a road in front: where to stop.
pub fn pick_house(terrain: &Terrain, rng: &mut Rng, from: Vec2, range: (f32, f32)) -> Option<Vec2> {
    let houses = &terrain.buildings;
    for _ in 0..400 {
        let house = &houses[rng.below(houses.len())];
        let middle = house.outline.iter().copied().sum::<Vec2>() / house.outline.len() as f32;
        let distance = middle.distance(from);
        if distance < range.0 || distance > range.1 {
            continue;
        }
        let Some(footprint) = Footprint::fit(&house.outline) else {
            continue;
        };
        if !(50.0..400.0).contains(&footprint.area()) {
            continue;
        }
        let (at, _) = kerb(terrain, middle);
        if at.distance(middle) < 30.0 && terrain.inside(at.x, at.y) {
            return Some(at);
        }
    }
    None
}

/// Starts the job on offer: its marker goes up and it's on the map.
pub fn take_offer(
    jobs: &mut Jobs,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    terrain: &Terrain,
) {
    let Some(mut job) = jobs.offer.take() else {
        return;
    };
    job.marker = Some(spawn_marker(
        commands,
        meshes,
        materials,
        terrain,
        job.destination,
        DESTINATION,
    ));
    jobs.current = Some(job);
    jobs.since_hit = HIT_GAP;
}

/// Counts the hits of the car being driven and keeps the HUD's line about the job.
#[allow(clippy::too_many_arguments)]
fn watch(
    time: Res<Time>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut jobs: ResMut<Jobs>,
    mut status: ResMut<Status>,
    mut news: ResMut<News>,
    places: Res<Places>,
    mut marks: ResMut<MapMarks>,
) {
    let dt = time.delta_secs();
    let knocks = cars
        .iter_mut()
        .find(|car| car.index == active.0)
        .map_or(0.0, |mut car| std::mem::take(&mut car.knocks));
    jobs.since_hit += dt;
    let since_hit = jobs.since_hit;
    status.0.clear();
    marks.0.truncate(places.list.len());
    let Some(job) = &mut jobs.current else {
        return;
    };
    job.elapsed += dt;
    let mut hit = false;
    if knocks > HIT && since_hit > HIT_GAP {
        job.hits += 1;
        hit = true;
        news.say(match job.kind {
            JobKind::Pizza => "Ouch! That pizza won't look the same.",
        });
    }
    let distance = cars
        .iter()
        .find(|car| car.index == active.0)
        .map_or(0.0, |car| car.body.position.xz().distance(job.destination));
    let what = match job.kind {
        JobKind::Pizza => "Pizza delivery",
    };
    let mut line = format!(
        "{what}: {:.1} km, {}",
        distance / 1000.0,
        euros(job.earned())
    );
    if job.kind == JobKind::Pizza && job.hits == 0 && job.time.is_some_and(|t| job.elapsed <= t) {
        line += " with the tip";
    }
    if let Some(limit) = job.time {
        let left = limit - job.elapsed;
        if left >= 0.0 {
            line += &format!(", {}:{:02} left", (left / 60.0) as u32, left as u32 % 60);
        } else {
            line += ", late!";
        }
    }
    if job.hits > 0 {
        line += &format!(", {} hit{}", job.hits, if job.hits == 1 { "" } else { "s" });
    }
    status.0.push(line);
    marks.0.push(MapMark {
        at: job.destination,
        color: DESTINATION,
        label: what.to_string(),
    });
    if hit {
        jobs.since_hit = 0.0;
    }
}

/// Stopping on the destination's marker finishes the job.
fn deliver(
    mut commands: Commands,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut jobs: ResMut<Jobs>,
    mut career: ResMut<Career>,
    mut news: ResMut<News>,
) {
    let Some(job) = &jobs.current else {
        return;
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let arrived = car.body.position.xz().distance(job.destination) < MARKER_RADIUS
        && car.body.velocity.length() < STOPPED;
    if !arrived || active.0 != PLAYER {
        return;
    }
    let job = jobs.current.take().expect("a job");
    if let Some(marker) = job.marker {
        commands.entity(marker).despawn();
    }
    let earned = job.earned();
    career.money += earned;
    match job.kind {
        JobKind::Pizza => {
            career.pizzas += 1;
            let tip = if job.hits == 0 && job.time.is_some_and(|t| job.elapsed <= t) {
                " with a tip"
            } else {
                ""
            };
            news.say(format!("Pizza delivered: {}{tip}.", euros(earned)));
        }
    }
    career.save();
}

/// Leaving the road (into the garage, or towed) ends any job unfinished.
fn give_up(mut commands: Commands, mut jobs: ResMut<Jobs>, mut news: ResMut<News>) {
    jobs.offer = None;
    let Some(job) = jobs.current.take() else {
        return;
    };
    if let Some(marker) = job.marker {
        commands.entity(marker).despawn();
    }
    news.say(match job.kind {
        JobKind::Pizza => "The pizza went cold. No pay.",
    });
}

/// An arrow at the top of the screen pointing to the destination, with how far it is.
fn arrow(
    mut contexts: EguiContexts,
    jobs: Res<Jobs>,
    cameras: Query<&GlobalTransform, With<ChaseCamera>>,
) -> Result {
    let (Some(job), Ok(eye)) = (&jobs.current, cameras.single()) else {
        return Ok(());
    };
    let forward = eye.forward().xz().normalize_or(Vec2::NEG_Y);
    let to = job.destination - eye.translation().xz();
    // Angle from straight ahead, clockwise (to the right) positive.
    let angle = forward
        .perp_dot(to.normalize_or_zero())
        .atan2(forward.dot(to.normalize_or_zero()));
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    let center = pos2(ctx.content_rect().center().x, 96.0);
    let turn = |p: egui::Vec2| {
        let (s, c) = angle.sin_cos();
        center + vec2(p.x * c - p.y * s, p.x * s + p.y * c)
    };
    let points = [
        vec2(0.0, -26.0),
        vec2(16.0, 14.0),
        vec2(0.0, 5.0),
        vec2(-16.0, 14.0),
    ];
    let fill = Color32::from_rgb(255, 255, 255);
    for tri in [[0, 1, 2], [0, 2, 3]] {
        let tri = tri.iter().map(|&i| turn(points[i])).collect();
        painter.add(Shape::convex_polygon(tri, fill, Stroke::NONE));
    }
    let outline = points.iter().map(|&p| turn(p)).collect();
    painter.add(Shape::closed_line(
        outline,
        Stroke::new(2.0, Color32::BLACK),
    ));
    let metres = to.length();
    let text = if metres > 1000.0 {
        format!("{:.1} km", metres / 1000.0)
    } else {
        format!("{metres:.0} m")
    };
    painter.text(
        center + vec2(0.0, 36.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(15.0),
        Color32::WHITE,
    );
    Ok(())
}

/// The job under way, with a button to give it up.
fn job_window(
    mut contexts: EguiContexts,
    mut commands: Commands,
    mut jobs: ResMut<Jobs>,
    mut news: ResMut<News>,
) -> Result {
    if jobs.current.is_none() {
        return Ok(());
    }
    let mut quit = false;
    egui::Window::new("Job")
        .anchor(Align2::RIGHT_TOP, vec2(-12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .show(contexts.ctx_mut()?, |ui| {
            quit = ui.button("Give up the job").clicked();
        });
    if quit && let Some(job) = jobs.current.take() {
        if let Some(marker) = job.marker {
            commands.entity(marker).despawn();
        }
        news.say(match job.kind {
            JobKind::Pizza => "You gave up the pizza. No pay.",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pizzas_go_to_houses_by_a_road() {
        let terrain = Terrain::baked();
        let places = Places::new(&terrain);
        let from = places.of(super::super::PlaceKind::Pizzeria).at;
        let mut jobs = Jobs::default();
        for _ in 0..10 {
            let job = jobs.pizza(&terrain, from).expect("a house for the pizza");
            assert!((PIZZA_RANGE.0 - 50.0..PIZZA_RANGE.1 + 50.0).contains(&job.distance));
            let (on_road, _) = kerb(&terrain, job.destination);
            assert!(on_road.distance(job.destination) < 1.0);
            assert!(job.pay > PIZZA_BASE && job.time.unwrap() > 60.0);
        }
    }

    #[test]
    fn late_or_knocked_pizzas_pay_less() {
        let mut job = Job {
            kind: JobKind::Pizza,
            destination: Vec2::ZERO,
            distance: 2000.0,
            pay: 28.0,
            time: Some(300.0),
            elapsed: 200.0,
            hits: 0,
            marker: None,
        };
        let best = job.earned();
        assert!((best - 28.0 * (1.0 + PIZZA_TIP)).abs() < 1e-6);
        job.hits = 1;
        assert!((job.earned() - 28.0).abs() < 1e-6);
        job.elapsed = 450.0;
        assert!((job.earned() - 14.0).abs() < 1e-6);
        job.elapsed = 3000.0;
        assert!((job.earned() - 28.0 * 0.3).abs() < 1e-6);
    }
}
