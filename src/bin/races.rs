//! The race editor: fly over the map, place a race's grid and checkpoints, export the
//! route for the server's events file (native only; no cars). `races [FILE]` opens FILE.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use bevy::prelude::*;
    use car_sim::*;

    let mut args = std::env::args().skip(1);
    let open = args.next().map(std::path::PathBuf::from);
    if args.next().is_some()
        || open
            .as_ref()
            .is_some_and(|p| p.to_string_lossy().starts_with('-'))
    {
        eprintln!("Usage: races [FILE]   (FILE: a route the race editor exported, to open)");
        std::process::exit(2);
    }
    App::new()
        .add_plugins(default_plugins("Car Sim: Race editor"))
        .add_plugins(race_editor::RaceEditorPlugin { open })
        .run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
