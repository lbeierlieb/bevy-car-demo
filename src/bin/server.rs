//! The dedicated server (native only): the world without anything to see or hear, run
//! headless. Players join it over the network (see "Multiplayer" in the notes).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use std::time::Duration;

    use bevy::app::ScheduleRunnerPlugin;
    use bevy::log::LogPlugin;
    use bevy::prelude::*;
    use car_sim::*;

    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
            LogPlugin::default(),
        ))
        .add_plugins((WorldPlugin, net::ServerPlugin))
        .add_systems(Update, report)
        .run();
}

/// Every minute, how many players and traffic cars there are.
#[cfg(not(target_arch = "wasm32"))]
fn report(
    time: bevy::prelude::Res<bevy::prelude::Time>,
    mut next: bevy::prelude::Local<f32>,
    players: bevy::prelude::Query<(), bevy::prelude::With<car_sim::traffic::Anchor>>,
    traffic: bevy::prelude::Query<(), bevy::prelude::With<car_sim::traffic::TrafficCar>>,
) {
    if time.elapsed_secs() >= *next {
        *next += 60.0;
        bevy::log::info!(
            "{} players, {} traffic cars",
            players.iter().count(),
            traffic.iter().count()
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
