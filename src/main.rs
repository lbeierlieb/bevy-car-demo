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
            // F1: gizmos, F2: the panel with telemetry and setup sliders.
            debug::DebugPlugin,
            ui::PanelPlugin,
            // The tool windows: ECU tune (F3), garage (F4), dyno (F5), showroom (F6).
            ecu_ui::EcuUiPlugin,
            garage::GaragePlugin,
            dyno::DynoPlugin,
        ))
        .run();
}
