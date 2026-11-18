//! The map editor: fly over the map, edit the map file, re-bake (native only; no cars).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy::prelude::*;
    use car_sim::*;

    App::new()
        .add_plugins(default_plugins("Car Sim: Map editor"))
        .add_plugins(editor::EditorPlugin)
        .run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
