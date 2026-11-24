//! The race editor (`bin/races.rs`, native only, no cars): fly over the map like in the
//! map editor and make a race's route for the server's events file. Grid spots are
//! placed and turned by hand (a box as big as the biggest car, turning with the mouse
//! until the next click); checkpoints are clicked onto the roads, each a gate, shown as
//! the race will show it, along the way the server will find. A click on the first
//! checkpoint makes the route a loop. Discs mark what was clicked and can be dragged;
//! Ctrl+click puts a checkpoint into the route between two others. Export writes a file
//! to paste under a `race` line; Open reads one back (or the file on the command line).
//!
//! Pieces: `edit` (mouse and keys), `show` (the boxes, discs, gates and the route in the
//! world and on the map), `panel` (the window and the open/export dialogs).

pub mod edit;
pub mod panel;
pub mod show;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use crate::events::race::{self, GridSpot, Route};
use crate::events::routes::Roads;
use crate::terrain::Terrain;

/// The route is found again at most this often while something's being dragged or
/// turned (s).
const RESOLVE_EVERY: f64 = 0.1;

pub struct RaceEditorPlugin {
    /// A file to open at the start.
    pub open: Option<PathBuf>,
}

impl Plugin for RaceEditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            brightness: 250.0,
            ..default()
        })
        .insert_resource(Draft::default())
        .insert_resource(crate::editor::camera::SpeedScale(3.0))
        .insert_resource(OpenAtStart(self.open.clone()))
        .add_plugins((
            crate::terrain::TerrainPlugin,
            crate::scenery::SceneryPlugin,
            crate::sky::SkyPlugin,
            crate::map::MapPlugin,
            EguiPlugin::default(),
            // Keys go to a text field being typed into; the pointer is checked by hand
            // (`edit`), as in the map editor, so a drag isn't dropped over the window.
            crate::ui::ShieldPlugin { pointer: false },
            crate::editor::camera::FlyCameraPlugin,
            edit::EditPlugin,
            show::ShowPlugin,
            panel::PanelPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, resolve.after(edit::EditSet));
    }
}

/// What's placed: the route without its name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub looped: bool,
    /// In the order placed (`Draft::pole_first` numbers them).
    pub grid: Vec<GridSpot>,
    pub checkpoints: Vec<Vec2>,
}

/// The route being made, its history, and how it goes along the roads.
#[derive(Resource, Default)]
pub struct Draft {
    pub plan: Plan,
    undo: Vec<Plan>,
    redo: Vec<Plan>,
    /// What the last change was, for changes merged into one step (turning a grid spot
    /// with the wheel, notch by notch).
    last_kind: Option<(u8, usize)>,
    /// The route's name.
    pub name: String,
    /// The file it came from or went to last (in the working directory, unless a path).
    pub file: String,
    /// Counts every change.
    pub revision: u64,
    /// The route found along the roads, as of `found`.
    pub route: Route,
    found: u64,
    /// When it was last found (s, real time).
    found_at: f64,
    /// What the server would say against it.
    pub problem: Option<String>,
    /// Something to tell (opened, written, couldn't…), and whether it's bad.
    pub note: Option<(String, bool)>,
}

impl Draft {
    /// Changes the plan, as one undo step.
    pub fn change(&mut self, f: impl FnOnce(&mut Plan)) {
        self.last_kind = None;
        let before = self.plan.clone();
        f(&mut self.plan);
        if self.plan != before {
            self.undo.push(before);
            self.redo.clear();
            self.revision += 1;
        }
    }

    /// Changes the plan, merged into the last undo step if that was a change of the same
    /// `kind`.
    pub fn change_merged(&mut self, kind: (u8, usize), f: impl FnOnce(&mut Plan)) {
        let merge = self.last_kind == Some(kind);
        let before = self.plan.clone();
        f(&mut self.plan);
        if self.plan != before {
            if !merge {
                self.undo.push(before);
                self.redo.clear();
            }
            self.revision += 1;
        }
        self.last_kind = Some(kind);
    }

    /// The plan was changed in place (while dragging or turning): shown, not yet a step.
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// A drag or turn that began at `before` is done: one undo step, if anything changed.
    pub fn commit(&mut self, before: Plan) {
        self.last_kind = None;
        if self.plan != before {
            self.undo.push(before);
            self.redo.clear();
        }
        self.revision += 1;
    }

    /// A drag or turn that began at `before` is called off.
    pub fn cancel(&mut self, before: Plan) {
        self.plan = before;
        self.revision += 1;
    }

    pub fn undo(&mut self) {
        if let Some(plan) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.plan, plan));
            self.last_kind = None;
            self.revision += 1;
        }
    }

    pub fn redo(&mut self) {
        if let Some(plan) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.plan, plan));
            self.last_kind = None;
            self.revision += 1;
        }
    }

    /// The revision the route was last found for.
    pub fn found(&self) -> u64 {
        self.found
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The grid spots' indices, pole first (nearest the first checkpoint, as the server
    /// orders them; in the order placed without one).
    pub fn pole_first(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.plan.grid.len()).collect();
        if let Some(&first) = self.plan.checkpoints.first() {
            let distance = |i: usize| self.plan.grid[i].at.distance(first);
            order.sort_by(|&a, &b| distance(a).total_cmp(&distance(b)));
        }
        order
    }

    /// The route as it would be exported (its line not found).
    pub fn route(&self) -> Route {
        let mut grid = self.plan.grid.clone();
        if let Some(&first) = self.plan.checkpoints.first() {
            race::order_grid(&mut grid, first);
        }
        Route {
            name: self.name.trim().to_string(),
            looped: self.plan.looped && self.plan.checkpoints.len() >= 2,
            grid,
            checkpoints: self.plan.checkpoints.clone(),
            line: Vec::new(),
            widths: Vec::new(),
            marks: Vec::new(),
        }
    }

    /// Reads the first route in `path` (what Export writes, or an events list).
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut routes = crate::events::read_routes(&text)?;
        if routes.is_empty() {
            return Err("there's no route in it".into());
        }
        let more = routes.len() - 1;
        let route = routes.swap_remove(0);
        let plan = Plan {
            looped: route.looped,
            grid: route.grid,
            checkpoints: route.checkpoints,
        };
        self.undo.push(std::mem::replace(&mut self.plan, plan));
        self.redo.clear();
        self.last_kind = None;
        self.revision += 1;
        self.name = route.name;
        self.file = path.display().to_string();
        let others = match more {
            0 => String::new(),
            1 => " (the one other route in it was left out)".into(),
            n => format!(" (the {n} other routes in it were left out)"),
        };
        self.note = Some((format!("Opened \"{}\"{others}.", self.name), false));
        Ok(())
    }

    /// What Export writes: a comment, a race line to uncomment for a new event (its marker
    /// at pole), and the route.
    pub fn text(&self) -> String {
        let route = self.route();
        let pole = route.grid.first().map_or(Vec2::ZERO, |spot| spot.at);
        format!(
            "# Made with the race editor (cargo run --bin races). Paste it under a race in the\n\
             # events file; for a new event, uncomment the race line too (its marker at pole).\n\
             # race  {:.0}  {:.0}  {}\n{}",
            pole.x,
            pole.y,
            route.name,
            route.text()
        )
    }
}

/// The file in the working directory a name given in a dialog means (with `.cfg` added
/// if it has no extension), or the path itself if it has a directory.
pub fn file_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(name.trim());
    if path.extension().is_none() {
        path.set_extension("cfg");
    }
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

/// The roads, as the server finds routes along them.
#[derive(Resource)]
pub struct RoadGraph(pub Roads);

#[derive(Resource)]
struct OpenAtStart(Option<PathBuf>);

fn setup(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut map: ResMut<crate::map::MapView>,
    mut draft: ResMut<Draft>,
    mut aim: ResMut<edit::AimAt>,
    open: Res<OpenAtStart>,
) {
    crate::spawn_sun(&mut commands);
    map.key = KeyCode::KeyM;
    map.click = Some("fly there");
    let started = std::time::Instant::now();
    commands.insert_resource(RoadGraph(Roads::new(&terrain)));
    info!(
        "The roads' graph took {:.0} ms",
        started.elapsed().as_secs_f32() * 1000.0
    );
    draft.name = "New route".into();
    // Found (and found wanting) at once.
    draft.touch();
    if let Some(path) = &open.0 {
        if let Err(e) = draft.open(path) {
            draft.note = Some((format!("Couldn't open {}: {e}", path.display()), true));
        }
        aim.0 = draft.plan.checkpoints.first().copied();
    }
}

/// Finds the route along the roads again after a change (at most every `RESOLVE_EVERY`
/// while the plan keeps changing), and says what the server would say against it.
fn resolve(mut draft: ResMut<Draft>, roads: Option<Res<RoadGraph>>, time: Res<Time<Real>>) {
    let Some(roads) = roads else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if draft.found == draft.revision || now - draft.found_at < RESOLVE_EVERY {
        return;
    }
    let mut route = draft.route();
    let problem = crate::events::resolve_route(&roads.0, &mut route).err();
    draft.problem = problem.or_else(|| {
        (route.grid.len() > race::MAX_RACERS)
            .then(|| format!("{} grid spots at most", race::MAX_RACERS))
    });
    draft.route = route;
    draft.found = draft.revision;
    draft.found_at = now;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_export_writes_opens_again_pole_first() {
        let mut draft = Draft {
            name: "Round the block".into(),
            ..default()
        };
        draft.plan = Plan {
            looped: true,
            grid: vec![
                GridSpot {
                    at: Vec2::new(-20.0, 0.0),
                    heading: 90.0,
                },
                GridSpot {
                    at: Vec2::new(-8.25, 1.5),
                    heading: 92.5,
                },
            ],
            checkpoints: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(300.0, 0.0),
                Vec2::new(300.0, 300.0),
            ],
        };
        let text = draft.text();
        assert!(text.contains("# race  -8  2  Round the block"), "{text}");
        let path = std::env::temp_dir().join(format!("race-editor-{}.cfg", std::process::id()));
        std::fs::write(&path, &text).unwrap();
        let mut back = Draft::default();
        back.open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.name, "Round the block");
        assert!(back.plan.looped);
        assert_eq!(back.plan.checkpoints, draft.plan.checkpoints);
        // Pole (nearest the first checkpoint) first, to a tenth of a metre.
        assert_eq!(back.plan.grid[0].at, Vec2::new(-8.2, 1.5));
        assert_eq!(back.plan.grid[1].at, Vec2::new(-20.0, 0.0));
        // The events list takes it under a race.
        let list = format!("race 0 0 Races\n{text}");
        assert_eq!(crate::events::parse(&list).unwrap()[0].routes.len(), 1);
    }
}
