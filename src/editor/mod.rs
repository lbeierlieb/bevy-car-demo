//! The map editor (`bin/editor.rs`, native only): fly over the baked map and edit the map
//! file (`map/gerach.osm`) it was baked from, then re-bake and see the result in place.
//! No cars. Agreed design in CLAUDE.md ("Map editor").
//!
//! - `doc`: the map file with undo/redo.
//! - `camera`: the fly camera, and teleporting from the map (M).
//! - `index`: what the map file's ways and nodes are, and where (for drawing and picking).
//! - `overlay`: the map file drawn over the baked map, and what's under the cursor.
//! - `tools`: selecting, moving, drawing, placing, deleting, heights.
//! - `panel`: the side panel (inspector, tags, tools, file, conflicts).
//! - `bake`: saving, running the bake, and reloading the map.
//! - `conflicts`: the bake's list of problems (`map/gerach.conflicts.gpx`) in 3D.

pub mod bake;
pub mod camera;
pub mod conflicts;
pub mod doc;
pub mod index;
pub mod overlay;
pub mod panel;
pub mod tools;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

use crate::osm::Frame;

/// The map file, from the repo's root.
pub const MAP_FILE: &str = "map/gerach.osm";
/// The bake's problem list it writes next to the map file.
pub const CONFLICTS_FILE: &str = "map/gerach.conflicts.gpx";
/// The map rectangle: its centre (latitude, longitude) and size in metres, as the bake
/// takes them (README, "The map and baking it").
pub const CENTER: (f64, f64) = (50.0118569, 10.8455963);
pub const SIZE: &str = "9600x8448";

/// A file of the repo: from the working directory if it's there (`cargo run` from the
/// repo), else in the source tree the editor was built from.
pub fn repo_file(path: &str) -> PathBuf {
    let here = PathBuf::from(path);
    if here.exists() {
        here
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
    }
}

/// The map frame the bake uses.
pub fn frame() -> Frame {
    Frame::around(CENTER.0, CENTER.1)
}

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            brightness: 250.0,
            ..default()
        })
        .add_plugins((
            crate::terrain::TerrainPlugin,
            crate::scenery::SceneryPlugin,
            crate::sky::SkyPlugin,
            crate::map::MapPlugin,
            EguiPlugin::default(),
            crate::ui::ShieldPlugin { pointer: false },
        ))
        .add_plugins((
            camera::FlyCameraPlugin,
            index::IndexPlugin,
            overlay::OverlayPlugin,
            tools::ToolsPlugin,
            panel::PanelPlugin,
            bake::BakePlugin,
            conflicts::ConflictsPlugin,
        ))
        .add_systems(PreStartup, load)
        .add_systems(Startup, setup);
    }
}

/// Reads the map file; without it there's nothing to edit.
fn load(mut commands: Commands, mut map: ResMut<crate::map::MapView>) {
    let path = repo_file(MAP_FILE);
    let started = std::time::Instant::now();
    match doc::Doc::load(path.clone(), frame()) {
        Ok(doc) => {
            info!(
                "Read {} in {:.1} s: {} nodes, {} ways",
                path.display(),
                started.elapsed().as_secs_f32(),
                doc.osm.nodes.len(),
                doc.osm.ways.len()
            );
            commands.insert_resource(doc);
        }
        Err(e) => panic!("Couldn't read the map file {}: {e}", path.display()),
    }
    map.key = KeyCode::KeyM;
    map.click = Some("fly there");
}

fn setup(mut commands: Commands) {
    // egui's pointer input isn't held back (that would drop a node being dragged as soon
    // as the pointer crosses the panel): the systems ask `EguiWantsInput` themselves.
    crate::spawn_sun(&mut commands);
}
