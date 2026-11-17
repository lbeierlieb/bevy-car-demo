//! The career: start with mom's car, earn money with deliveries, buy cars and parts.

use bevy::prelude::*;
use car_sim::*;

fn main() {
    App::new()
        .add_plugins(default_plugins("Car Sim: Career"))
        .add_plugins(CorePlugin)
        .add_plugins(career::CareerPlugin)
        .run();
}
