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
            // The events a server puts on the map: a car hunt (none alone).
            events::EventsPlugin,
            // Starts in the coupé; R puts the car back on the nearest road.
            car::DefaultCarPlugin,
            car::ResetPlugin,
            // 1: gizmos, 2: the panel with telemetry, driving aids and bindings (hidden
            // at first).
            debug::DebugPlugin,
            ui::PanelPlugin,
            // The garage, the showroom, the chassis dyno and the ECU tuning (in it), and the
            // alignment lift are at shops.
            garage::GaragePlugin,
            dyno::DynoPlugin,
            alignment::AlignmentPlugin,
        ))
        .run();
}
