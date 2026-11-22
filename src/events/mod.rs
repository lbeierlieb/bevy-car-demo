//! Events on the map: places to drive to for a game with other players (a car hunt, races
//! and time trials). Like the shops, the server says where they are (its own list,
//! `--events`, e.g. `assets/events.cfg`; none without one) when a player joins; playing
//! alone there are none. Each has a marker like a shop's, its name floating above it. A
//! race's routes are in the list too, under it, as the points they pass through; the
//! server finds the way along the roads between them (`resolve`) and sends the lines.

pub mod hunt;
pub mod lobby;
pub mod race;
pub mod round;
pub mod rounds;
pub mod routes;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::ChaseCamera;
use crate::map::{MapLegend, MapMark, MapMarks};
use crate::net::wire::{Reader, Wire};
use crate::net::{Connection, Status};
use crate::shops::{float_label, spawn_marker};
use crate::terrain::Terrain;

/// The repo's list (`assets/events.cfg`), for tests: the server has none of its own.
#[cfg(test)]
pub const LIST: &str = include_str!("../../assets/events.cfg");

/// On the map (M), event names show when zoomed in to fewer metres per screen point than
/// this (farther out than the shops': there are few).
const MAP_NAMES_BELOW: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKind {
    /// One player hunted, the others hunting.
    Hunt,
    /// Races and time trials on its routes.
    Race,
}

impl EventKind {
    pub const ALL: [EventKind; 2] = [Self::Hunt, Self::Race];

    /// Its word in a list of events.
    pub fn word(self) -> &'static str {
        match self {
            Self::Hunt => "hunt",
            Self::Race => "race",
        }
    }

    /// What it is, for people.
    pub fn title(self) -> &'static str {
        match self {
            Self::Hunt => "Car hunt",
            Self::Race => "Races",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Hunt => [255, 50, 50],
            Self::Race => [60, 200, 255],
        }
    }
}

/// An event as the server lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub name: String,
    /// Where its marker is (map x and z), exactly (unlike a shop's, it isn't moved to a
    /// road: a parking lot is a good place for one).
    pub at: Vec2,
    /// A race's routes (none for other kinds).
    pub routes: Vec<race::Route>,
}

/// Reads a list of events: one per line, its kind, x, z and name, with `#` comments and
/// blank lines skipped. Under a race come its routes: `route loop|sprint name`, then the
/// points it passes through, `x z` a line. The routes' lines along the roads are left
/// for `resolve`.
pub fn parse(text: &str) -> Result<Vec<Event>, String> {
    let mut events: Vec<Event> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let wrong = |what: &str| format!("line {}: {what}: {line}", n + 1);
        let mut words = line.split_whitespace();
        let word = words.next().unwrap_or("");
        // A route's point, under the route.
        if let Ok(x) = word.parse::<f32>() {
            let z = (words.next())
                .and_then(|w| w.parse::<f32>().ok())
                .filter(|z| z.is_finite() && x.is_finite() && words.next().is_none())
                .ok_or_else(|| wrong("a route's point is x and z (m), nothing more"))?;
            let route = (events.last_mut())
                .and_then(|event| event.routes.last_mut())
                .ok_or_else(|| wrong("a point, but no route above it"))?;
            route.via.push(Vec2::new(x, z));
            continue;
        }
        if word == "route" {
            let looped = match words.next() {
                Some("loop") => true,
                Some("sprint") => false,
                _ => return Err(wrong("a route is a loop or a sprint")),
            };
            let name = words.collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                return Err(wrong("a route needs a name"));
            }
            let event = (events.last_mut())
                .filter(|event| event.kind == EventKind::Race)
                .ok_or_else(|| wrong("a route, but no race above it"))?;
            if event.routes.len() >= 255 {
                return Err(wrong("too many routes"));
            }
            event.routes.push(race::Route {
                name,
                looped,
                via: Vec::new(),
                line: Vec::new(),
                widths: Vec::new(),
            });
            continue;
        }
        let kind = (EventKind::ALL.into_iter())
            .find(|kind| kind.word() == word)
            .ok_or_else(|| {
                let kinds: Vec<_> = EventKind::ALL.iter().map(|k| k.word()).collect();
                wrong(&format!("not a kind of event ({})", kinds.join(", ")))
            })?;
        let mut number = || {
            (words.next())
                .and_then(|w| w.parse::<f32>().ok())
                .filter(|x| x.is_finite())
                .ok_or_else(|| wrong("needs x and z (m) after the kind"))
        };
        let at = Vec2::new(number()?, number()?);
        let name = words.collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err(wrong("needs a name after x and z"));
        }
        events.push(Event {
            kind,
            name,
            at,
            routes: Vec::new(),
        });
    }
    if events.len() > usize::from(u16::MAX) {
        return Err("too many events".into());
    }
    for event in &events {
        if event.kind == EventKind::Race && event.routes.is_empty() {
            return Err(format!("{}: a race needs a route", event.name));
        }
        if let Some(route) = event.routes.iter().find(|route| route.via.len() < 2) {
            return Err(format!(
                "{}, {}: a route needs two points at least",
                event.name, route.name
            ));
        }
    }
    Ok(events)
}

/// Finds the races' routes along the roads (`routes`), or says what's wrong with one.
pub fn resolve(events: &mut [Event], terrain: &Terrain) -> Result<(), String> {
    if events.iter().all(|event| event.routes.is_empty()) {
        return Ok(());
    }
    let roads = routes::Roads::new(terrain);
    for event in events {
        for route in &mut event.routes {
            let wrong = |e: String| format!("{}, {}: {e}", event.name, route.name);
            let (line, widths) = roads.route(&route.via, route.looped).map_err(wrong)?;
            route.line = line;
            route.widths = widths;
            let length = route.length();
            if length < race::SHORTEST {
                return Err(wrong(format!(
                    "{length:.0} m long, shorter than {:.0} m",
                    race::SHORTEST
                )));
            }
        }
    }
    Ok(())
}

impl Wire for Event {
    fn put(&self, out: &mut Vec<u8>) {
        let kind = EventKind::ALL.iter().position(|&k| k == self.kind);
        (kind.unwrap_or(0) as u8).put(out);
        self.name.put(out);
        self.at.put(out);
        self.routes.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            kind: *EventKind::ALL.get(usize::from(r.get::<u8>()?))?,
            name: r.get()?,
            at: r.get()?,
            routes: r.get()?,
        })
    }
}

/// An event where the client put it.
pub struct Placed {
    pub event: Event,
    /// The marker's middle (map x and z): the event's point.
    pub marker: Vec2,
}

/// The events on the map: the server's, once joined; none alone.
#[derive(Resource, Default)]
pub struct Events {
    pub list: Vec<Placed>,
}

/// An event's marker.
#[derive(Component)]
pub struct Marker;

pub struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Events>()
            .add_plugins(lobby::LobbyPlugin)
            .add_systems(Update, place)
            .add_systems(EguiPrimaryContextPass, labels);
    }
}

/// Puts the server's events on the map (and the map, M) when they come, and takes them off
/// when we leave.
fn place(
    mut commands: Commands,
    connection: Option<Res<Connection>>,
    mut events: ResMut<Events>,
    markers: Query<Entity, With<Marker>>,
    terrain: Res<Terrain>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    (mut marks, mut legend): (ResMut<MapMarks>, ResMut<MapLegend>),
) {
    // None in a round under way: it's in a world of its own.
    let wanted: &[Event] = match connection.as_deref() {
        Some(c) if matches!(c.status, Status::Joined { .. }) && !c.in_running_round() => &c.events,
        _ => &[],
    };
    let current = events.list.iter().map(|placed| &placed.event);
    if current.eq(wanted.iter()) {
        return;
    }
    for marker in &markers {
        commands.entity(marker).despawn();
    }
    events.list = (wanted.iter())
        .map(|event| {
            let marker = event.at;
            let entity = spawn_marker(
                &mut commands,
                &mut meshes,
                &mut materials,
                &terrain,
                marker,
                event.kind.color(),
            );
            commands.entity(entity).insert(Marker);
            Placed {
                event: event.clone(),
                marker,
            }
        })
        .collect();
    let event_marks = (events.list.iter())
        .map(|placed| MapMark {
            at: placed.marker,
            color: placed.event.kind.color(),
            label: placed.event.name.clone(),
            label_below: MAP_NAMES_BELOW,
        })
        .collect();
    marks.0.insert("events", event_marks);
    let kinds = (EventKind::ALL.into_iter())
        .filter(|&kind| events.list.iter().any(|placed| placed.event.kind == kind))
        .map(|kind| (kind.color(), kind.title().to_string()))
        .collect();
    legend.0.insert("events", kinds);
}

/// The events' names above their markers, when they're near and in view.
fn labels(
    mut contexts: EguiContexts,
    events: Res<Events>,
    terrain: Res<Terrain>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    if events.list.is_empty() {
        return Ok(());
    }
    let Ok(camera) = cameras.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    for placed in &events.list {
        let kind = placed.event.kind;
        let label = (
            placed.marker,
            &*placed.event.name,
            kind.title(),
            kind.color(),
        );
        float_label(&painter, camera, &terrain, label);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::surface::Surface;

    #[test]
    fn the_list_reads() {
        let events = parse(LIST).unwrap();
        assert!(events.iter().any(|event| event.kind == EventKind::Hunt));
        assert!(
            parse("rally 1 2 Track")
                .unwrap_err()
                .starts_with("line 1: not a kind")
        );
        assert!(parse("race 1 2 Track").is_err(), "no route");
        assert!(parse("hunt 1 2 Hunt\nroute loop Round\n1 2\n3 4").is_err());
        assert!(
            parse("race 1 2 Track\nroute loop Round\n1 2").is_err(),
            "one point"
        );
        let race = parse("race 1 2 Track\nroute loop Round the block\n1 2\n3 4 # here\n5 6\nroute sprint Up\n7 8\n9 10")
            .unwrap()
            .remove(0);
        assert_eq!(race.routes.len(), 2);
        assert_eq!(race.routes[0].name, "Round the block");
        assert!(race.routes[0].looped && !race.routes[1].looped);
        assert_eq!(
            race.routes[0].via,
            [
                Vec2::new(1.0, 2.0),
                Vec2::new(3.0, 4.0),
                Vec2::new(5.0, 6.0)
            ]
        );
        assert!(parse("hunt 1 Hunt").is_err());
        assert!(parse("hunt 1 2").is_err());
    }

    #[test]
    fn the_races_routes_are_found_along_the_roads() {
        let terrain = Terrain::baked();
        let mut events = parse(LIST).unwrap();
        resolve(&mut events, &terrain).unwrap();
        let race = (events.iter())
            .find(|event| event.kind == EventKind::Race)
            .unwrap();
        assert!(race.routes.len() >= 2);
        for route in &race.routes {
            let length = route.length();
            println!("{}: {length:.0} m, {} points", route.name, route.line.len());
            assert!(length > race::SHORTEST, "{}", route.name);
            // Along the roads: every point on one.
            for &p in &route.line {
                let (on, _, _) = terrain.paved_road_near(p).unwrap();
                assert!(on.distance(p) < 1.0, "{} at {p}", route.name);
            }
            if route.looped {
                assert_eq!(route.line.first(), route.line.last());
            }
        }
        // A point off the roads, or one the route turns back at, is refused.
        let mut far = parse("race 0 0 R\nroute sprint S\n254 2566\n800 2480").unwrap();
        assert!(
            resolve(&mut far, &terrain)
                .unwrap_err()
                .contains("from the nearest road")
        );
        let mut back = parse("race 0 0 R\nroute sprint S\n254 2566\n518 2830\n254 2566").unwrap();
        assert!(
            resolve(&mut back, &terrain)
                .unwrap_err()
                .contains("turns back at point 2")
        );
    }

    #[test]
    fn every_event_is_on_paved_ground() {
        let terrain = Terrain::baked();
        for event in parse(LIST).unwrap() {
            let surface = terrain.surface(event.at.x, event.at.y);
            assert!(
                matches!(surface, Surface::Asphalt | Surface::Cobbles),
                "{} is on {surface:?}",
                event.name,
            );
        }
    }
}
