//! What the last bake found wrong (`map/gerach.conflicts.gpx`: roads still in buildings,
//! barriers on roads), as red posts in 3D, with next/previous in the panel.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use quick_xml::events::Event;

use super::camera::FlyCamera;
use super::{CONFLICTS_FILE, repo_file};
use crate::map::{MapMark, MapMarks, MapView};
use crate::osm::Frame;
use crate::terrain::Terrain;

/// Posts are drawn within this distance (m), labelled within the shorter one.
const SHOW_WITHIN: f32 = 800.0;
const LABEL_WITHIN: f32 = 150.0;
const POST: f32 = 12.0;
const RED: Color = Color::srgb(1.0, 0.15, 0.1);

pub struct ConflictsPlugin;

impl Plugin for ConflictsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut commands: Commands| {
            let mut conflicts = Conflicts {
                shown: true,
                ..default()
            };
            conflicts.reload(super::frame());
            commands.insert_resource(conflicts);
        })
        .add_systems(Update, (draw, mark))
        .add_systems(EguiPrimaryContextPass, labels);
    }
}

pub struct Conflict {
    pub at: Vec2,
    pub name: String,
    pub desc: String,
}

#[derive(Resource, Default)]
pub struct Conflicts {
    pub list: Vec<Conflict>,
    /// The one last jumped to.
    pub current: Option<usize>,
    pub shown: bool,
}

impl Conflicts {
    /// Reads the bake's list again (none if there's no file).
    pub fn reload(&mut self, frame: Frame) {
        let text = std::fs::read_to_string(repo_file(CONFLICTS_FILE)).unwrap_or_default();
        self.list = parse(&text, frame);
        self.current = None;
    }

    /// Steps to the next (or previous) one and returns it.
    pub fn step(&mut self, forward: bool) -> Option<&Conflict> {
        let n = self.list.len();
        if n == 0 {
            return None;
        }
        let next = match (self.current, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        self.current = Some(next);
        self.list.get(next)
    }
}

/// The waypoints of a GPX file.
fn parse(text: &str, frame: Frame) -> Vec<Conflict> {
    let mut out = Vec::new();
    let mut reader = quick_xml::Reader::from_str(text);
    let (mut at, mut field, mut name, mut desc) = (None, "", String::new(), String::new());
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "wpt" => {
                    let attr = |key: &str| {
                        e.attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == key)
                            .and_then(|a| a.value.parse::<f64>().ok())
                    };
                    at = attr("lat").zip(attr("lon"));
                    (name, desc) = (String::new(), String::new());
                }
                "name" => field = "name",
                "desc" => field = "desc",
                _ => field = "",
            },
            Ok(Event::Text(t)) => {
                let text: &str = t.as_ref();
                match field {
                    "name" => name += text,
                    "desc" => desc += text,
                    _ => {}
                }
            }
            Ok(Event::GeneralRef(r)) => {
                let text = match r.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "apos" => "'",
                    "quot" => "\"",
                    _ => "",
                };
                match field {
                    "name" => name += text,
                    "desc" => desc += text,
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                field = "";
                if e.name().as_ref() == "wpt"
                    && let Some((lat, lon)) = at.take()
                {
                    let (x, z) = frame.to_local(lat, lon);
                    out.push(Conflict {
                        at: Vec2::new(x as f32, z as f32),
                        name: std::mem::take(&mut name),
                        desc: std::mem::take(&mut desc),
                    });
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn draw(
    conflicts: Res<Conflicts>,
    terrain: Res<Terrain>,
    cameras: Query<&GlobalTransform, With<FlyCamera>>,
    mut gizmos: Gizmos,
) {
    let Ok(eye) = cameras.single() else {
        return;
    };
    if !conflicts.shown {
        return;
    }
    for (i, c) in conflicts.list.iter().enumerate() {
        if c.at.distance(eye.translation().xz()) > SHOW_WITHIN {
            continue;
        }
        let base = c.at.extend(terrain.sample(c.at.x, c.at.y).0).xzy();
        let color = if conflicts.current == Some(i) {
            Color::srgb(1.0, 0.9, 0.1)
        } else {
            RED
        };
        gizmos.line(base, base + Vec3::Y * POST, color);
        gizmos.sphere(base + Vec3::Y * POST, 0.8, color);
        gizmos.circle(
            Isometry3d::new(
                base + Vec3::Y * 0.3,
                Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            ),
            3.0,
            color,
        );
    }
}

/// The current one on the 2D map.
fn mark(conflicts: Res<Conflicts>, mut marks: ResMut<MapMarks>) {
    if !conflicts.is_changed() {
        return;
    }
    marks.0.clear();
    if let Some(c) = conflicts.current.and_then(|i| conflicts.list.get(i)) {
        marks.0.push(MapMark {
            at: c.at,
            color: [230, 40, 30],
            label: c.name.clone(),
        });
    }
}

fn labels(
    mut contexts: EguiContexts,
    conflicts: Res<Conflicts>,
    terrain: Res<Terrain>,
    view: Res<MapView>,
    cameras: Query<(&Camera, &GlobalTransform), With<FlyCamera>>,
) -> Result {
    let Ok((camera, eye)) = cameras.single() else {
        return Ok(());
    };
    if !conflicts.shown || view.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    for c in &conflicts.list {
        if c.at.distance(eye.translation().xz()) > LABEL_WITHIN {
            continue;
        }
        let top =
            c.at.extend(terrain.sample(c.at.x, c.at.y).0 + POST + 1.5)
                .xzy();
        let Ok(at) = camera.world_to_viewport(eye, top) else {
            continue;
        };
        let text = format!("{}: {}", c.name, c.desc);
        let galley = painter.layout(text, FontId::proportional(13.0), Color32::WHITE, 320.0);
        let rect = Align2::CENTER_BOTTOM.anchor_size(egui::pos2(at.x, at.y), galley.size());
        painter.rect_filled(
            rect.expand(4.0),
            3.0,
            Color32::from_rgba_premultiplied(90, 10, 5, 200),
        );
        painter.galley(rect.min, galley, Color32::WHITE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waypoints_are_read() {
        let gpx = "<?xml version='1.0'?><gpx><wpt lat='50.0139956' lon='10.8847176'>\
            <name>2.2 m</name><desc>way 1 &amp; building 2</desc></wpt></gpx>";
        let list = parse(gpx, super::super::frame());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "2.2 m");
        assert_eq!(list[0].desc, "way 1 & building 2");
        assert!(list[0].at.length() > 100.0);
    }
}
