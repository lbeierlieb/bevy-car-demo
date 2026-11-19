//! The dedicated server (native only): the world without anything to see or hear, run
//! headless, which players join over WebTransport (see "Multiplayer" in the notes).
//!
//! ```text
//! server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]
//! ```
//!
//! Without a certificate it makes a throwaway one: only the native game joins that, with
//! `CAR_SIM_INSECURE=1` (for development).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use std::time::Duration;

    use bevy::app::ScheduleRunnerPlugin;
    use bevy::log::LogPlugin;
    use bevy::prelude::*;
    use car_sim::net::quic::{Certificate, QuicListener};
    use car_sim::*;

    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(e) => {
            eprintln!(
                "{e}\n\nusage: server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]"
            );
            std::process::exit(2);
        }
    };
    let certificate = match (options.cert, options.key) {
        (Some(chain), Some(key)) => Certificate::Files { chain, key },
        _ => Certificate::Throwaway,
    };
    let throwaway = matches!(certificate, Certificate::Throwaway);
    let listener = match QuicListener::bind(options.port, certificate) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
        LogPlugin::default(),
    ))
    .add_plugins((WorldPlugin, net::ServerPlugin))
    .insert_resource(net::Server::new(listener).with_max_players(options.max_players))
    .insert_resource(traffic::Traffic {
        cars: options.traffic,
    })
    .add_systems(Update, report);
    if throwaway {
        warn!(
            "No certificate given (--cert, --key): using a throwaway one, which only the \
             native game joins, with CAR_SIM_INSECURE=1"
        );
    }
    info!("Listening on UDP port {}", options.port);
    app.run();
}

#[cfg(not(target_arch = "wasm32"))]
struct Options {
    port: u16,
    cert: Option<std::path::PathBuf>,
    key: Option<std::path::PathBuf>,
    traffic: usize,
    max_players: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Options {
            port: car_sim::net::PORT,
            cert: None,
            key: None,
            traffic: 20,
            max_players: 32,
        };
        fn number<T: std::str::FromStr>(arg: &str, value: Option<String>) -> Result<T, String> {
            let value = value.ok_or(format!("{arg} needs a value"))?;
            value
                .parse()
                .map_err(|_| format!("{arg}: not a number: {value}"))
        }
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--port" => options.port = number(&arg, args.next())?,
                "--cert" => options.cert = Some(args.next().ok_or("--cert needs a file")?.into()),
                "--key" => options.key = Some(args.next().ok_or("--key needs a file")?.into()),
                "--traffic" => {
                    options.traffic =
                        number::<usize>(&arg, args.next())?.min(car_sim::traffic::MOST_CARS)
                }
                "--max-players" => options.max_players = number(&arg, args.next())?,
                _ => return Err(format!("unknown option {arg}")),
            }
        }
        if options.cert.is_some() != options.key.is_some() {
            return Err("--cert and --key go together".into());
        }
        Ok(options)
    }
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
            "{} players driving, {} traffic cars",
            players.iter().count(),
            traffic.iter().count()
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
