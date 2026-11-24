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
            // Esc, Start or the wheel's menu button: everything else.
            menu::central::CentralMenuPlugin,
            // The shops a server puts on the map (none alone).
            shops::ShopsPlugin,
            // The events a server puts on the map: a car hunt (none alone).
            events::EventsPlugin,
            // Starts in the coupé; R puts the car back on the nearest road.
            car::DefaultCarPlugin,
            // 1: gizmos, 2: the panel with telemetry, driving aids and bindings (hidden
            // at first).
            debug::DebugPlugin,
            ui::PanelPlugin,
            // The garage (on its lift), the showroom, the chassis dyno and the ECU tuning (in it), the
            // alignment lift, the pumps and the paint booth are at shops.
            garage::GaragePlugin,
            dyno::DynoPlugin,
            alignment::AlignmentPlugin,
            gas::GasPlugin,
            paint::PaintPlugin,
        ))
        .run();
}
