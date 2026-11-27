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
            // The shops a server puts on the map (none alone), and at each the trunk and
            // the parts kept there (the garage's at the garage park).
            shops::ShopsPlugin,
            trunk::TrunkPlugin,
            // The shops that trade: used parts.
            dealers::DealersPlugin,
            // The events a server puts on the map: a car hunt (none alone).
            events::EventsPlugin,
            // (The car is the server's to give: `net::client::take_your_car`.)
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
