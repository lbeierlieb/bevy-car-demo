//! The sandbox: every car and part for free, with the developer tools; alone or on a
//! server.

use bevy::prelude::*;
use car_sim::*;

fn main() {
    App::new()
        .add_plugins(default_plugins("Car Sim"))
        .add_plugins(CorePlugin)
        .add_plugins((
            // The controls at the start (H), and hints when the car won't go.
            help::HelpPlugin,
            // The start menu (after the controls): play alone or join a server.
            online::OnlinePlugin,
            // The shops a server puts on the map (none alone).
            shops::ShopsPlugin,
            // Starts in the coupé; R puts the car back on the nearest road as new.
            car::DefaultCarPlugin,
            car::ResetPlugin,
            // 1: gizmos, 2: the panel with telemetry, setup sliders and bindings (hidden
            // at first).
            debug::DebugPlugin,
            ui::PanelPlugin,
            // The ECU tune (3) and the engine dyno (5); the garage and the showroom are at
            // shops, as is the chassis dyno.
            ecu_ui::EcuUiPlugin,
            garage::GaragePlugin,
            dyno::DynoPlugin,
            dyno::engine::EngineDynoPlugin,
        ))
        .run();
}
