//! The dedicated server (native only): the world without anything to see or hear, run
//! headless, which players join over WebTransport (see "Multiplayer" in the notes).
//!
//! ```text
//! server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]
//!        [--map-players self|near|all] [--map-near 200] [--chat on|off]
//!        [--name-tags on|off] [--shops shops.cfg]
//!        [--telegram-token-file token --telegram-chat-file chat]
//! ```
//!
//! `--map-players` says who a player sees of the others on their map: nobody (`self`),
//! the ones within `--map-near` metres (`near`, the default) or everyone (`all`).
//! `--chat off` keeps the players from chatting (it's on by default), `--name-tags off`
//! the players' names off their cars (also on by default). `--shops` reads where the
//! shops are from a file (see `assets/shops.cfg`, the list it has otherwise).
//!
//! Without a certificate it makes a throwaway one: only the native game joins that, with
//! `CAR_SIM_INSECURE=1` (for development).
//!
//! With a Telegram bot's token and a chat (each in a file, so both stay secret and out
//! of the command line), the bot tells that chat who joins and leaves. The chat is a
//! user's, group's or channel's id (or `@channel`); the bot must be in it, and a user
//! must have written to the bot first.

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
                "{e}\n\nusage: server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]\n       [--map-players self|near|all] [--map-near 200] [--chat on|off]\n       [--name-tags on|off] [--shops shops.cfg]\n       [--telegram-token-file token --telegram-chat-file chat]"
            );
            std::process::exit(2);
        }
    };
    let certificate = match (options.cert, options.key) {
        (Some(chain), Some(key)) => Certificate::Files { chain, key },
        _ => Certificate::Throwaway,
    };
    let throwaway = matches!(certificate, Certificate::Throwaway);
    let telegram = match (options.telegram_token_file, options.telegram_chat_file) {
        (Some(token), Some(chat)) => match (secret(&token), secret(&chat)) {
            (Ok(token), Ok(chat)) => Some(Telegram::start(&token, chat)),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => None,
    };
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
    .insert_resource(
        net::Server::new(listener)
            .with_max_players(options.max_players)
            .with_map(options.map_players)
            .with_chat(options.chat)
            .with_name_tags(options.name_tags)
            .with_shops(options.shops),
    )
    .insert_resource(traffic::Traffic {
        cars: options.traffic,
    })
    .add_systems(Update, (report, timing));
    if let Some(telegram) = telegram {
        telegram.send("The server started".to_string());
        app.insert_resource(telegram)
            .add_systems(Update, tell_telegram);
    }
    if throwaway {
        warn!(
            "No certificate given (--cert, --key): using a throwaway one, which only the \
             native game joins, with CAR_SIM_INSECURE=1"
        );
    }
    // One thread for the systems: there's little to do in each, and handing it to the
    // task pool's threads every frame cost more than the work (an idle server: 2.3 % of a
    // core, 0.6 % so; with traffic 5.5 %, 3.1 %); the VPS it runs on has one core anyway.
    for (_, schedule) in app
        .world_mut()
        .resource_mut::<bevy::ecs::schedule::Schedules>()
        .iter_mut()
    {
        schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
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
    map_players: car_sim::net::MapPlayers,
    chat: bool,
    name_tags: bool,
    shops: Vec<car_sim::shops::Shop>,
    telegram_token_file: Option<std::path::PathBuf>,
    telegram_chat_file: Option<std::path::PathBuf>,
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
            map_players: car_sim::net::MapPlayers::Near(car_sim::net::server::MAP_NEAR),
            chat: true,
            name_tags: true,
            shops: Vec::new(),
            telegram_token_file: None,
            telegram_chat_file: None,
        };
        fn number<T: std::str::FromStr>(arg: &str, value: Option<String>) -> Result<T, String> {
            let value = value.ok_or(format!("{arg} needs a value"))?;
            value
                .parse()
                .map_err(|_| format!("{arg}: not a number: {value}"))
        }
        use car_sim::net::MapPlayers;
        let mut map_players = "near".to_string();
        let mut map_near = car_sim::net::server::MAP_NEAR;
        let mut shops: Option<std::path::PathBuf> = None;
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
                "--map-players" => {
                    map_players = args.next().ok_or("--map-players needs self, near or all")?
                }
                "--map-near" => map_near = number(&arg, args.next())?,
                "--shops" => shops = Some(args.next().ok_or("--shops needs a file")?.into()),
                "--chat" | "--name-tags" => {
                    let on = match args.next().as_deref() {
                        Some("on") => true,
                        Some("off") => false,
                        _ => return Err(format!("{arg} needs on or off")),
                    };
                    if arg == "--chat" {
                        options.chat = on;
                    } else {
                        options.name_tags = on;
                    }
                }
                "--telegram-token-file" => {
                    options.telegram_token_file = Some(
                        args.next()
                            .ok_or("--telegram-token-file needs a file")?
                            .into(),
                    )
                }
                "--telegram-chat-file" => {
                    options.telegram_chat_file = Some(
                        args.next()
                            .ok_or("--telegram-chat-file needs a file")?
                            .into(),
                    )
                }
                _ => return Err(format!("unknown option {arg}")),
            }
        }
        options.map_players = match map_players.as_str() {
            "self" => MapPlayers::Nobody,
            "near" => MapPlayers::Near(map_near),
            "all" => MapPlayers::Everyone,
            other => return Err(format!("--map-players: self, near or all, not {other}")),
        };
        if !(map_near.is_finite() && map_near >= 0.0) {
            return Err(format!("--map-near: not a distance: {map_near}"));
        }
        let text = match &shops {
            Some(file) => std::fs::read_to_string(file)
                .map_err(|e| format!("--shops: {}: {e}", file.display()))?,
            None => car_sim::shops::DEFAULT.to_string(),
        };
        options.shops = car_sim::shops::parse(&text).map_err(|e| format!("--shops: {e}"))?;
        if options.cert.is_some() != options.key.is_some() {
            return Err("--cert and --key go together".into());
        }
        if options.telegram_token_file.is_some() != options.telegram_chat_file.is_some() {
            return Err("--telegram-token-file and --telegram-chat-file go together".into());
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

/// Whether the server keeps up: every 10 s that it didn't (a frame took over 40 ms, or the
/// simulation fell behind the clock by more than a step's overstep), the longest frame
/// and how far behind.
#[cfg(not(target_arch = "wasm32"))]
fn timing(
    real: bevy::prelude::Res<bevy::prelude::Time<bevy::time::Real>>,
    fixed: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Fixed>>,
    mut state: bevy::prelude::Local<Option<Timing>>,
) {
    let now = real.elapsed_secs_f64();
    let behind = now - fixed.elapsed_secs_f64();
    let state = state.get_or_insert(Timing::new(now, behind));
    let dt = real.delta_secs_f64();
    state.longest = state.longest.max(dt);
    state.slow += usize::from(dt > 0.04);
    state.frames += 1;
    if now - state.since >= 10.0 {
        let lost = behind - state.behind;
        if state.slow > 0 || lost > 0.02 {
            bevy::log::warn!(
                "Last {:.0} s: {} of {} frames over 40 ms, the longest {:.0} ms; simulation \
                 fell {:.0} ms behind the clock",
                now - state.since,
                state.slow,
                state.frames,
                state.longest * 1000.0,
                lost * 1000.0
            );
        }
        *state = Timing::new(now, behind);
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Timing {
    since: f64,
    /// The clock less the simulation's time when this stretch began (s).
    behind: f64,
    longest: f64,
    slow: usize,
    frames: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Timing {
    fn new(since: f64, behind: f64) -> Self {
        Self {
            since,
            behind,
            longest: 0.0,
            slow: 0,
            frames: 0,
        }
    }
}

/// What a file holds, trimmed (a secret: a token or chat id), or why not.
#[cfg(not(target_arch = "wasm32"))]
fn secret(file: &std::path::Path) -> Result<String, String> {
    match std::fs::read_to_string(file) {
        Ok(text) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        Ok(_) => Err(format!("{}: empty", file.display())),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// A Telegram bot telling a chat who joins and leaves. The messages go out one after
/// another from a thread of their own, so a slow or unreachable Telegram never holds up
/// the game; one that fails is logged and dropped.
#[cfg(not(target_arch = "wasm32"))]
#[derive(bevy::prelude::Resource)]
struct Telegram {
    messages: std::sync::mpsc::Sender<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Telegram {
    fn start(token: &str, chat: String) -> Self {
        let token = token.to_string();
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let (messages, inbox) = std::sync::mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("telegram".into())
            .spawn(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(std::time::Duration::from_secs(10)))
                    .build()
                    .into();
                for text in inbox {
                    let sent = agent
                        .post(&url)
                        .send_form([("chat_id", chat.as_str()), ("text", text.as_str())]);
                    if let Err(e) = sent {
                        // Never the token in the log (a bad URL error would have it).
                        let e = e.to_string().replace(&token, "<token>");
                        bevy::log::warn!("Couldn't tell Telegram \"{text}\": {e}");
                    }
                }
            })
            .expect("a thread for Telegram");
        Self { messages }
    }

    fn send(&self, text: String) {
        let _ = self.messages.send(text);
    }
}

/// Tells Telegram who joined and who left.
#[cfg(not(target_arch = "wasm32"))]
fn tell_telegram(
    telegram: bevy::prelude::Res<Telegram>,
    mut presence: bevy::prelude::MessageReader<car_sim::net::Presence>,
) {
    for p in presence.read() {
        let players = match p.players {
            1 => "1 player".to_string(),
            n => format!("{n} players"),
        };
        let what = if p.joined { "joined" } else { "left" };
        telegram.send(format!("{} {what} ({players} on the server)", p.name));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
