//! A player's side of races and time trials: the question on a race's marker (time trial
//! or race), the lobby's window (route, direction, laps, traffic, contact, resets; who's
//! in; the best times; the last results), the bar at the top while it runs (countdown,
//! lap, place, times, the next gate, wrong way), the gates on the road and the route on
//! the map (M).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::lobby::{EventState, elapsed, leave, you};
use super::race::{self, COUNTDOWN, Course, LAPS, Results, clock};
use super::round::{Action, Game, Outcome, Phase, Round};
use super::{EventKind, Events};
use crate::car::{ActiveCar, Car};
use crate::map::{MapLine, MapLines, MapMark, MapMarks};
use crate::net::Connection;
use crate::terrain::Terrain;

/// The gates' posts: how tall, how thick, how far past the road's edge (m); the banner's
/// height and depth.
const POST_HEIGHT: f32 = 5.0;
const POST_RADIUS: f32 = 0.2;
const POST_OUT: f32 = 1.0;
const BANNER: f32 = 0.7;
/// Gates shown ahead: the next and this many more.
const GATES_AHEAD: usize = 2;
/// Going against the course faster than this (m/s), for this long (s), is the wrong way.
const WRONG_SPEED: f32 = 3.0;
const WRONG_TIME: f64 = 1.5;
/// "Go!" stays this long after the countdown (s).
const GO_SHOWN: f64 = 1.5;
/// The next gate's words in the bar, and its arrow's size (points).
const GATE_TEXT: Color32 = Color32::from_rgb(80, 255, 80);
const ARROW: f32 = 26.0;
/// The route on the map: its colour, the next gate's.
const ROUTE_COLOR: [u8; 3] = [60, 200, 255];
const NEXT_COLOR: [u8; 3] = [80, 255, 80];

pub struct RacingPlugin;

impl Plugin for RacingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GateLooks>()
            .add_systems(
                Update,
                (gates, map_route, wrong_way)
                    .after(super::lobby::follow)
                    .run_if(resource_exists::<Connection>),
            )
            .add_systems(EguiPrimaryContextPass, (choose, window));
    }
}

/// The gates on the road, for the course they're of.
#[derive(Resource, Default)]
struct GateLooks {
    /// The course's line (to notice another), and the gates' entities, a lap's.
    shown: Option<(Vec<Vec2>, Vec<Entity>)>,
    /// The banners' materials: the next gate's, the ones after, the finish.
    materials: Option<[Handle<StandardMaterial>; 3]>,
}

/// A gate's banner (its posts are its children's siblings: the gate's entity is their
/// parent).
#[derive(Component)]
struct Banner;

/// Our place in the race under way: the round, how far we are, the course.
fn ours<'a>(
    connection: &Connection,
    state: &'a mut EventState,
) -> Option<(Round, usize, &'a Course)> {
    let you = you(connection)?;
    let round = connection.round.clone().filter(Round::running)?;
    let passed = (round.standing()?.racers.iter())
        .find(|r| r.player == you)
        .map_or(0, |racer| usize::from(racer.gates));
    let course = state.course(connection, &round)?;
    Some((round, passed, course))
}

/// Puts the course's gates on the road while a race runs (the next one and a couple
/// after it shown, the next one lit), and takes them off after.
#[allow(clippy::too_many_arguments)]
fn gates(
    mut commands: Commands,
    connection: Res<Connection>,
    mut state: ResMut<EventState>,
    mut looks: ResMut<GateLooks>,
    terrain: Res<Terrain>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    mut visibility: Query<&mut Visibility>,
    children: Query<&Children>,
    mut banners: Query<&mut MeshMaterial3d<StandardMaterial>, With<Banner>>,
) {
    let ours = ours(&connection, &mut state);
    let line = ours.as_ref().map(|(_, _, course)| &course.line);
    if looks.shown.as_ref().map(|(l, _)| l) != line {
        if let Some((_, entities)) = looks.shown.take() {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
        if let Some((_, _, course)) = &ours {
            let [next, _, _] = looks
                .materials
                .get_or_insert_with(|| {
                    let mut glow = |r: u8, g: u8, b: u8, alpha: f32| {
                        let color = Color::srgb_u8(r, g, b);
                        materials.add(StandardMaterial {
                            base_color: color.with_alpha(alpha),
                            emissive: color.to_linear() * 1.5,
                            alpha_mode: AlphaMode::Blend,
                            unlit: true,
                            ..default()
                        })
                    };
                    [
                        glow(80, 255, 80, 0.85),
                        glow(255, 255, 255, 0.4),
                        glow(255, 210, 40, 0.9),
                    ]
                })
                .clone();
            let post_material = materials.add(StandardMaterial {
                base_color: Color::srgb(0.9, 0.9, 0.9),
                ..default()
            });
            let post = meshes.add(Cylinder::new(POST_RADIUS, POST_HEIGHT));
            let entities = (course.gates.iter())
                .map(|gate| {
                    let half = gate.width / 2.0 + POST_OUT;
                    let side = Vec2::new(-gate.forward.y, gate.forward.x);
                    let ground = |p: Vec2| terrain.ground(p.x, p.y, f32::INFINITY).height;
                    let (left, right) = (gate.point - side * half, gate.point + side * half);
                    let base = ground(left).max(ground(right));
                    let yaw = gate.yaw();
                    let banner = meshes.add(Cuboid::new(2.0 * half, BANNER, 0.1));
                    commands
                        .spawn((
                            Transform::from_xyz(gate.point.x, base, gate.point.y)
                                .with_rotation(Quat::from_rotation_y(yaw)),
                            Visibility::Hidden,
                        ))
                        .with_children(|parent| {
                            for x in [-half, half] {
                                parent.spawn((
                                    Mesh3d(post.clone()),
                                    MeshMaterial3d(post_material.clone()),
                                    Transform::from_xyz(x, POST_HEIGHT / 2.0, 0.0),
                                ));
                            }
                            parent.spawn((
                                Mesh3d(banner),
                                MeshMaterial3d(next.clone()),
                                Transform::from_xyz(0.0, POST_HEIGHT - BANNER / 2.0, 0.0),
                                NotShadowCaster,
                                Banner,
                            ));
                        })
                        .id()
                })
                .collect();
            looks.shown = Some((course.line.clone(), entities));
        }
        return;
    }
    let (Some((_, passed, course)), Some((_, entities)), Some(materials)) =
        (&ours, &looks.shown, &looks.materials)
    else {
        return;
    };
    let n = entities.len();
    let total = course.total();
    for (i, &entity) in entities.iter().enumerate() {
        // How many gates ahead this one is (0: the next).
        let ahead = (i + n - passed % n) % n;
        let shown = ahead <= GATES_AHEAD && passed + ahead < total;
        if let Ok(mut v) = visibility.get_mut(entity) {
            let wanted = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            v.set_if_neq(wanted);
        }
        let material = if passed + ahead + 1 == total {
            &materials[2]
        } else if ahead == 0 {
            &materials[0]
        } else {
            &materials[1]
        };
        for child in children.iter_descendants(entity) {
            if let Ok(mut banner) = banners.get_mut(child)
                && banner.0 != *material
            {
                banner.0 = material.clone();
            }
        }
    }
}

/// The route on the map (M): the race's under way, or the one picked in the lobby; the
/// next gate a mark.
fn map_route(
    connection: Res<Connection>,
    mut state: ResMut<EventState>,
    mut lines: ResMut<MapLines>,
    mut marks: ResMut<MapMarks>,
) {
    let round = connection.round.clone();
    let line = round.as_ref().and_then(|round| {
        let settings = round.game.race()?;
        let event = connection.events.get(usize::from(round.event))?;
        Some(&event.routes.get(usize::from(settings.route))?.line)
    });
    let wanted: Vec<MapLine> = line
        .map(|points| MapLine {
            points: points.clone(),
            color: ROUTE_COLOR,
            width: 4.0,
        })
        .into_iter()
        .collect();
    let same = (lines.0.get("race")).is_some_and(|have| {
        have.len() == wanted.len() && have.iter().zip(&wanted).all(|(a, b)| a.points == b.points)
    });
    if !same {
        lines.0.insert("race", wanted);
    }
    let next = ours(&connection, &mut state).and_then(|(_, passed, course)| {
        (passed < course.total()).then(|| MapMark {
            at: course.next(passed).point,
            color: NEXT_COLOR,
            label: "Next gate".into(),
            label_below: f32::INFINITY,
        })
    });
    match next {
        Some(mark) => {
            marks.0.insert("race", vec![mark]);
        }
        None => {
            marks.0.remove("race");
        }
    }
}

/// Notices our car going the wrong way round (against the course near where it should
/// be: between the gate last passed and the next).
fn wrong_way(
    connection: Res<Connection>,
    mut state: ResMut<EventState>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    let car = cars.iter().find(|car| car.index == active.0);
    let against = ours(&connection, &mut state)
        .zip(car)
        .and_then(|((_, passed, course), car)| {
            let next = course.next(passed).s;
            let last = if passed % course.gates.len() == 0 {
                0.0
            } else {
                course.next(passed - 1).s
            };
            let p = car.body.position.xz();
            let (_, forward) = course.nearest(p, last - 50.0, next + 50.0)?;
            let velocity = car.body.velocity.xz();
            Some(velocity.length() > WRONG_SPEED && velocity.normalize().dot(forward) < -0.3)
        });
    if against == Some(true) {
        state.wrong_way.get_or_insert(now);
    } else {
        state.wrong_way = None;
    }
}

/// On a race's marker: time trial or race?
fn choose(
    mut contexts: EguiContexts,
    connection: Option<ResMut<Connection>>,
    events: Res<Events>,
    mut state: ResMut<EventState>,
    time: Res<Time<Real>>,
) -> Result {
    let Some(mut connection) = connection else {
        return Ok(());
    };
    let Some(i) = state.choosing.filter(|_| connection.round.is_none()) else {
        return Ok(());
    };
    let Some(placed) = events
        .list
        .get(i)
        .filter(|p| p.event.kind == EventKind::Race)
    else {
        state.choosing = None;
        return Ok(());
    };
    let event = placed.event.clone();
    let ctx = contexts.ctx_mut()?;
    let racing = connection
        .busy
        .values()
        .filter(|&&e| usize::from(e) == i)
        .count();
    let mut action = None;
    let mut leaving = false;
    egui::Window::new(&event.name)
        .id(egui::Id::new("race choice"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("race routes")
                .num_columns(3)
                .show(ui, |ui| {
                    for route in &event.routes {
                        ui.label(&route.name);
                        ui.label(route.kind());
                        ui.label(format!("{:.1} km", route.length() / 1000.0));
                        ui.end_row();
                    }
                });
            ui.separator();
            if ui
                .button("Time trial")
                .on_hover_text("On your own, against the clock")
                .clicked()
            {
                action = Some(Action::Trial(i as u16));
            }
            let race = if racing > 0 {
                format!("Race ({racing} here)")
            } else {
                "Race (open a lobby)".into()
            };
            if ui
                .button(race)
                .on_hover_text("Against other players: whoever opens the lobby sets the race")
                .clicked()
            {
                action = Some(Action::Enter(i as u16));
            }
            leaving = ui
                .button("Leave (Esc)")
                .on_hover_text("Or drive off (Space releases the parking brake)")
                .clicked();
        });
    if let Some(action) = action {
        state.asked = time.elapsed_secs_f64();
        connection.act(action);
        state.choosing = None;
    }
    if leaving {
        state.choosing = None;
        state.left = Some(i);
    }
    Ok(())
}

/// A race's or time trial's lobby window, or the bar at the top while it runs.
fn window(
    mut contexts: EguiContexts,
    connection: Option<ResMut<Connection>>,
    mut state: ResMut<EventState>,
    time: Res<Time<Real>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
) -> Result {
    let Some(mut connection) = connection else {
        return Ok(());
    };
    let (Some(you), Some(round)) = (you(&connection), connection.round.clone()) else {
        return Ok(());
    };
    let Some(settings) = round.game.race().copied() else {
        return Ok(());
    };
    let Some(event) = connection.events.get(usize::from(round.event)).cloned() else {
        return Ok(());
    };
    let trial = matches!(round.game, Game::Trial(_));
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs_f64();
    let name = |id: u32| {
        if id == you {
            format!("{} (you)", connection.name)
        } else {
            connection.players.get(&id).cloned().unwrap_or_default()
        }
    };
    let host = round.host == you;
    match &round.phase {
        Phase::Lobby { last } => {
            let mut wanted = settings;
            let mut action = None;
            let mut leaving = false;
            let title = format!("{} ({})", event.name, round.game.title());
            egui::Window::new(title)
                .id(egui::Id::new("event"))
                .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some(Outcome::Race(results)) = last {
                        results_table(ui, results);
                        ui.separator();
                    }
                    if !trial {
                        egui::Grid::new("racers").num_columns(2).show(ui, |ui| {
                            for &id in &round.players {
                                ui.label(name(id));
                                if id == round.host {
                                    ui.label("Sets the race");
                                } else if host {
                                    if ui.button("Let them set it").clicked() {
                                        action = Some(Action::Host(id));
                                    }
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });
                        ui.separator();
                    }
                    ui.add_enabled_ui(host, |ui| {
                        egui::Grid::new("race settings")
                            .num_columns(2)
                            .show(ui, |ui| settings_grid(ui, &mut wanted, &event, trial));
                    });
                    if !host {
                        ui.small(format!("{} sets these.", name(round.host)));
                    }
                    if !round.best.is_empty() {
                        ui.separator();
                        let what = match event.routes.get(usize::from(settings.route)) {
                            Some(route) if route.looped => "Best laps",
                            _ => "Best times",
                        };
                        ui.label(RichText::new(format!("{what} (time trials)")).strong());
                        egui::Grid::new("best").num_columns(3).show(ui, |ui| {
                            for (i, best) in round.best.iter().enumerate() {
                                ui.label(format!("{}.", i + 1));
                                ui.label(&best.name);
                                ui.label(clock(best.time));
                                ui.end_row();
                            }
                        });
                    }
                    ui.separator();
                    if trial {
                        if ui.button("Start").clicked() {
                            action = Some(Action::Start);
                        }
                    } else if !host {
                        ui.label(format!(
                            "Waiting for {} to start the race.",
                            name(round.host)
                        ));
                    } else if round.players.len() < 2 {
                        ui.label(
                            "Waiting for another racer: anyone stopping on the marker can join.",
                        );
                    } else if ui.button("Start the race").clicked() {
                        action = Some(Action::Start);
                    }
                    leaving = ui
                        .button("Leave (Esc)")
                        .on_hover_text("Or drive off (Space releases the parking brake)")
                        .clicked();
                });
            if wanted != settings {
                let game = if trial {
                    Game::Trial(wanted)
                } else {
                    Game::Race(wanted)
                };
                // Shown at once, not only once the server has it.
                if let Some(round) = connection.round.as_mut() {
                    round.game = game;
                }
                connection.act(Action::Game(game));
            }
            if let Some(action) = action {
                connection.act(action);
            }
            if leaving {
                leave(&mut connection, &mut state);
            }
        }
        Phase::Running { start, .. } => {
            let elapsed = elapsed(&connection, &state, *start, now);
            let car = cars.iter().find(|car| car.index == active.0).cloned();
            let lines = race_lines(&connection, &mut state, &round, elapsed, now, car.as_ref());
            let mut action = None;
            let mut leaving = false;
            egui::Area::new(egui::Id::new("race bar"))
                .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(Color32::from_black_alpha(180))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                for line in &lines {
                                    line.show(ui);
                                }
                                ui.horizontal(|ui| {
                                    if trial && ui.small_button("Start again").clicked() {
                                        action = Some(Action::Restart);
                                    }
                                    let leave = if trial { "Stop" } else { "Leave the race" };
                                    leaving = ui.small_button(leave).clicked();
                                });
                            });
                        });
                });
            if let Some(action) = action {
                connection.act(action);
            }
            if leaving {
                leave(&mut connection, &mut state);
            }
        }
    }
    Ok(())
}

/// A line of the bar: text in a colour and size, or an arrow turned this far clockwise
/// from straight up (rad).
struct Line {
    text: String,
    color: Color32,
    size: f32,
    arrow: Option<f32>,
}

impl Line {
    fn new(text: String, color: Color32, size: f32) -> Self {
        Self {
            text,
            color,
            size,
            arrow: None,
        }
    }

    fn arrow(angle: f32) -> Self {
        Self {
            text: String::new(),
            color: GATE_TEXT,
            size: ARROW,
            arrow: Some(angle),
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        let Some(angle) = self.arrow else {
            ui.label(RichText::new(&self.text).size(self.size).color(self.color));
            return;
        };
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(self.size, self.size), egui::Sense::hover());
        let c = rect.center();
        let r = self.size / 2.0;
        let along = egui::vec2(angle.sin(), -angle.cos());
        let across = egui::vec2(-along.y, along.x);
        let points = vec![
            c + along * r,
            c - along * r * 0.6 + across * r * 0.7,
            c - along * r * 0.2,
            c - along * r * 0.6 - across * r * 0.7,
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            self.color,
            egui::Stroke::NONE,
        ));
    }
}

/// The bar's lines while a race runs.
fn race_lines(
    connection: &Connection,
    state: &mut EventState,
    round: &Round,
    elapsed: f64,
    now: f64,
    car: Option<&Car>,
) -> Vec<Line> {
    let white = Color32::WHITE;
    let mut lines = Vec::new();
    let Some(you) = you(connection) else {
        return lines;
    };
    let (Some(standing), Some(settings)) = (round.standing(), round.game.race()) else {
        return lines;
    };
    let trial = matches!(round.game, Game::Trial(_));
    let wrong_way = state.wrong_way;
    let gate_times = state.gate_times.clone();
    let Some(course) = state.course(connection, round) else {
        return lines;
    };
    let route = (connection.events.get(usize::from(round.event)))
        .and_then(|event| event.routes.get(usize::from(settings.route)))
        .map(|route| route.name.clone())
        .unwrap_or_default();
    let Some(me) = standing.racers.iter().find(|r| r.player == you) else {
        return lines;
    };
    let passed = usize::from(me.gates);
    let total = course.total();
    let racing = (elapsed - COUNTDOWN) as f32;
    // The countdown, and "Go!".
    if elapsed < COUNTDOWN {
        let left = (COUNTDOWN - elapsed).ceil() as u32;
        let text = if left > 3 {
            "Get ready".into()
        } else {
            format!("{left}")
        };
        lines.push(Line::new(text, Color32::from_rgb(255, 80, 60), 40.0));
    } else if elapsed < COUNTDOWN + GO_SHOWN && me.finished.is_none() {
        lines.push(Line::new(
            "Go!".into(),
            Color32::from_rgb(80, 255, 80),
            40.0,
        ));
    }
    let direction = if settings.reverse {
        ", the other way"
    } else {
        ""
    };
    lines.push(Line::new(format!("{route}{direction}"), white, 16.0));
    if let Some(time) = me.finished {
        let place = standing
            .order()
            .iter()
            .position(|&i| standing.racers[i].player == you);
        let text = match place {
            Some(place) if !trial => format!("Finished {}. in {}", place + 1, clock(time)),
            _ => format!("Finished in {}", clock(time)),
        };
        lines.push(Line::new(text, Color32::from_rgb(255, 210, 40), 24.0));
        if let (Some(closes), Some(server)) = (standing.closes, connection.server_time(now)) {
            let left = (closes - server).max(0.0) as u32;
            lines.push(Line::new(
                format!("Waiting for the others: {}:{:02}", left / 60, left % 60),
                white,
                16.0,
            ));
        }
        return lines;
    }
    // Lap, place, the clock.
    let mut top = Vec::new();
    if course.looped {
        let lap = (passed / course.gates.len() + 1).min(usize::from(course.laps));
        top.push(format!("Lap {lap}/{}", course.laps));
    }
    if !trial {
        let order = standing.order();
        let place = order.iter().position(|&i| standing.racers[i].player == you);
        top.push(format!(
            "{}. of {}",
            place.map_or(0, |p| p + 1),
            standing.racers.len()
        ));
    }
    top.push(clock(racing.max(0.0)));
    lines.push(Line::new(top.join("    "), white, 22.0));
    // Laps' times.
    if course.looped && !me.laps.is_empty() {
        let last = clock(*me.laps.last().unwrap());
        let best = clock(me.best_lap().unwrap_or_default());
        lines.push(Line::new(
            format!("Last lap {last}    Best {best}"),
            white,
            16.0,
        ));
    }
    // The gap to the one ahead, at the last gate we both passed.
    if !trial && passed > 0 {
        let order = standing.order();
        let place = order.iter().position(|&i| standing.racers[i].player == you);
        let ahead = place
            .filter(|&p| p > 0)
            .map(|p| &standing.racers[order[p - 1]]);
        if let Some(ahead) = ahead
            && let Some(theirs) = gate_times.get(&(ahead.player, me.gates))
        {
            let gap = (me.at - theirs).max(0.0);
            lines.push(Line::new(
                format!("{} ahead by {gap:.1} s", ahead.name),
                white,
                16.0,
            ));
        }
    }
    // The next gate: how far, which way.
    if let Some(car) = car
        && passed < total
    {
        let gate = course.next(passed);
        let p = car.body.position.xz();
        let to = gate.point - p;
        let forward = (car.body.rotation * Vec3::NEG_Z)
            .xz()
            .normalize_or(Vec2::NEG_Y);
        // Clockwise from straight ahead, seen from above (z south).
        let angle = forward
            .perp_dot(to.normalize_or(forward))
            .atan2(forward.dot(to.normalize_or(forward)));
        lines.push(Line::arrow(angle));
        let what = if passed + 1 == total {
            "Finish"
        } else {
            "Next gate"
        };
        let gates = format!("{}/{}", passed % course.gates.len(), course.gates.len());
        lines.push(Line::new(
            format!("{what} {:.0} m    Gates {gates}", to.length()),
            GATE_TEXT,
            16.0,
        ));
    }
    if wrong_way.is_some_and(|since| now - since > WRONG_TIME) {
        lines.push(Line::new("Wrong way!".into(), Color32::RED, 28.0));
    }
    lines
}

/// The last round's results, a row each.
fn results_table(ui: &mut egui::Ui, results: &Results) {
    let direction = if results.reverse {
        " (the other way)"
    } else {
        ""
    };
    ui.label(
        RichText::new(format!("{}{direction}", results.route))
            .strong()
            .size(16.0),
    );
    egui::Grid::new("results").num_columns(4).show(ui, |ui| {
        ui.label("");
        ui.label("");
        ui.label("Time");
        ui.label("Best lap");
        ui.end_row();
        for (i, entry) in results.entries.iter().enumerate() {
            ui.label(format!("{}.", i + 1));
            ui.label(&entry.name);
            ui.label(match entry.time {
                Some(time) => clock(time),
                None if entry.left => "Left".into(),
                None => "Didn't finish".into(),
            });
            ui.label(entry.best_lap.map(clock).unwrap_or_default());
            ui.end_row();
        }
    });
}

/// The settings, a row each.
fn settings_grid(
    ui: &mut egui::Ui,
    settings: &mut race::Settings,
    event: &super::Event,
    trial: bool,
) {
    ui.label("Route");
    let route = event.routes.get(usize::from(settings.route));
    let shown = route.map_or(String::new(), |route| {
        format!(
            "{} ({}, {:.1} km)",
            route.name,
            route.kind().to_lowercase(),
            route.length() / 1000.0
        )
    });
    egui::ComboBox::from_id_salt("race route")
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (i, route) in event.routes.iter().enumerate() {
                let label = format!("{} ({})", route.name, route.kind().to_lowercase());
                ui.selectable_value(&mut settings.route, i as u8, label);
            }
        });
    ui.end_row();
    ui.label("Direction");
    ui.horizontal(|ui| {
        ui.radio_value(&mut settings.reverse, false, "Normal");
        ui.radio_value(&mut settings.reverse, true, "Reverse");
    });
    ui.end_row();
    if route.is_some_and(|route| route.looped) {
        ui.label("Laps");
        ui.add(egui::Slider::new(&mut settings.laps, LAPS.0..=LAPS.1));
        ui.end_row();
    }
    ui.label("Traffic");
    ui.checkbox(&mut settings.traffic, "");
    ui.end_row();
    if !trial {
        ui.label("Cars collide");
        ui.checkbox(&mut settings.collisions, "")
            .on_hover_text("Off: the racers' cars pass through each other (drawn see-through); traffic still counts");
        ui.end_row();
        ui.label("Racers can reset (R)");
        ui.checkbox(&mut settings.resets, "").on_hover_text(
            "A helicopter carries the car back to the last gate passed: it takes a while",
        );
        ui.end_row();
    }
}
