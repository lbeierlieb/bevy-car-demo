//! Events on the map: places to drive to for a game with other players (a car hunt; races
//! to come). Like the shops, the server says where they are (its own list, `--events`,
//! e.g. `assets/events.cfg`; none without one) when a player joins; playing alone there
//! are none. Each has a marker on the road like a shop's, its name floating above it.

pub mod hunt;
pub mod lobby;
pub mod rounds;

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
}

impl EventKind {
    pub const ALL: [EventKind; 1] = [Self::Hunt];

    /// Its word in a list of events.
    pub fn word(self) -> &'static str {
        match self {
            Self::Hunt => "hunt",
        }
    }

    /// What it is, for people.
    pub fn title(self) -> &'static str {
        match self {
            Self::Hunt => "Car hunt",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Hunt => [255, 50, 50],
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
}

/// Reads a list of events: one per line, its kind, x, z and name, with `#` comments and
/// blank lines skipped.
pub fn parse(text: &str) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let wrong = |what: &str| format!("line {}: {what}: {line}", n + 1);
        let mut words = line.split_whitespace();
        let word = words.next().unwrap_or("");
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
        events.push(Event { kind, name, at });
    }
    if events.len() > usize::from(u16::MAX) {
        return Err("too many events".into());
    }
    Ok(events)
}

impl Wire for Event {
    fn put(&self, out: &mut Vec<u8>) {
        let kind = EventKind::ALL.iter().position(|&k| k == self.kind);
        (kind.unwrap_or(0) as u8).put(out);
        self.name.put(out);
        self.at.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            kind: *EventKind::ALL.get(usize::from(r.get::<u8>()?))?,
            name: r.get()?,
            at: r.get()?,
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
            parse("race 1 2 Track")
                .unwrap_err()
                .starts_with("line 1: not a kind")
        );
        assert!(parse("hunt 1 Hunt").is_err());
        assert!(parse("hunt 1 2").is_err());
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
