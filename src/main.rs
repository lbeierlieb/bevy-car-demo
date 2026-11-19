//! The sandbox: every car and part for free, with the developer tools.

use bevy::prelude::*;
use car_sim::*;

fn main() {
    App::new()
        .add_plugins(default_plugins("Car Sim"))
        .add_plugins(CorePlugin)
        .add_plugins((
            // Starts in the coupé; R puts the car back on its wheels as new.
            car::DefaultCarPlugin,
            car::ResetPlugin,
            // A click on the map (K) drives from there.
            map::TeleportPlugin,
            // 1: gizmos, 2: the panel with telemetry and setup sliders.
            debug::DebugPlugin,
            ui::PanelPlugin,
            // The tool windows: ECU tune (3), garage (4), dyno (5), showroom (6).
            ecu_ui::EcuUiPlugin,
            garage::GaragePlugin,
            dyno::DynoPlugin,
        ))
        .run();
}
