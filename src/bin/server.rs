//! The dedicated server (native only): the world without anything to see or hear, run
//! headless, which players join over WebTransport (see "Multiplayer" in the notes).
//!
//! ```text
//! server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]
//!        [--map-players self|near|all] [--map-near 200] [--chat on|off]
//!        [--name-tags on|off] [--shops shops.cfg] [--events events.cfg]
//!        [--spawn "x z [heading]"] [--telegram-token-file token --telegram-chat-file chat]
//!        [--database URL | --database-file file] [--guests on|off]
//! server --database URL user add|password|remove NAME
//! server --database URL user list
//! ```
//!
//! `--map-players` says who a player sees of the others on their map: nobody (`self`),
//! the ones within `--map-near` metres (`near`, the default) or everyone (`all`).
//! `--chat off` keeps the players from chatting (it's on by default), `--name-tags off`
//! the players' names off their cars (also on by default). `--shops` reads where the
//! shops are from a file (e.g. `assets/shops.cfg`; without one there are none), `--events`
//! where the events are (e.g. `assets/events.cfg`; likewise). `--spawn` is where players
//! start: a map point (x east, z south, m, as the gizmos (1) and the map show them) and a
//! compass heading to face along the road there (without one, the way with more road
//! behind); without it, the game's default in Gerach.
//!
//! With `--database` (`sqlite:///var/lib/car-sim/car-sim.db`, a plain path to an SQLite
//! file, `mysql://user:password@host/db` or `postgres://user:password@host/db`) players log
//! in to accounts and their cars are kept; `--guests on` lets in players without one too
//! (by a name that isn't an account's, without a password; nothing of theirs is kept).
//! Accounts are made by the server's admin: `user add NAME` asks for the password, `user
//! password NAME` sets a new one, `user remove NAME` deletes the account and what it has,
//! `user list` lists them (a password is read from the terminal, or a line on standard
//! input). `--database-file` reads the URL from a file (a secret, when it has a password).
//! Several servers may share a PostgreSQL or MySQL database: a player plays on one
//! at a time. Without a database everyone plays by the name they give, and nothing's kept.
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
    use bevy::prelude::*;
    use car_sim::net::quic::{Certificate, QuicListener};
    use car_sim::*;

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(at) = args.iter().position(|arg| arg == "user") {
        std::process::exit(users(&args[..at], &args[at + 1..]));
    }
    let options = match Options::parse(args.into_iter()) {
        Ok(options) => options,
        Err(e) => {
            eprintln!(
                "{e}\n\nusage: server [--port 4433] [--cert fullchain.pem --key key.pem] [--traffic 20] [--max-players 32]\n       [--map-players self|near|all] [--map-near 200] [--chat on|off]\n       [--name-tags on|off] [--shops shops.cfg] [--events events.cfg]\n       [--spawn \"x z [heading]\"] [--telegram-token-file token --telegram-chat-file chat]\n       [--database URL | --database-file file] [--guests on|off]\n       server --database URL user add|password|remove NAME | user list"
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
    let accounts = match &options.database {
        Some(url) => match net::accounts::Accounts::open(url, options.guests) {
            Ok(accounts) => Some(accounts),
            Err(e) => {
                eprintln!("--database: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    let listener = match QuicListener::bind(options.port, certificate) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let mut server = net::Server::new(listener);
    if let Some(accounts) = accounts {
        server = server.with_accounts(accounts);
    }
    let mut app = net::server_app(
        server
            .with_max_players(options.max_players)
            .with_map(options.map_players)
            .with_chat(options.chat)
            .with_name_tags(options.name_tags)
            .with_shops(options.shops)
            .with_spawn(options.spawn)
            .with_events(options.events),
        options.traffic,
        true,
    );
    app.add_systems(Update, (report, timing));
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
    info!("Listening on UDP port {}", options.port);
    net::run_server(app);
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
    events: Vec<car_sim::events::Event>,
    spawn: car_sim::terrain::Spawn,
    telegram_token_file: Option<std::path::PathBuf>,
    telegram_chat_file: Option<std::path::PathBuf>,
    database: Option<String>,
    guests: bool,
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
            events: Vec::new(),
            spawn: car_sim::terrain::Spawn::DEFAULT,
            telegram_token_file: None,
            telegram_chat_file: None,
            database: None,
            guests: false,
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
        let mut events: Option<std::path::PathBuf> = None;
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
                "--events" => events = Some(args.next().ok_or("--events needs a file")?.into()),
                "--spawn" => {
                    let text = args.next().ok_or("--spawn needs \"x z [heading]\"")?;
                    options.spawn = car_sim::terrain::Spawn::parse(&text)
                        .map_err(|e| format!("--spawn: {e}"))?;
                }
                "--database" => {
                    options.database = Some(args.next().ok_or("--database needs a URL")?)
                }
                "--database-file" => {
                    let file = args.next().ok_or("--database-file needs a file")?;
                    options.database = Some(secret(std::path::Path::new(&file))?);
                }
                "--chat" | "--name-tags" | "--guests" => {
                    let on = match args.next().as_deref() {
                        Some("on") => true,
                        Some("off") => false,
                        _ => return Err(format!("{arg} needs on or off")),
                    };
                    match arg.as_str() {
                        "--chat" => options.chat = on,
                        "--guests" => options.guests = on,
                        _ => options.name_tags = on,
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
        if let Some(file) = &shops {
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("--shops: {}: {e}", file.display()))?;
            options.shops = car_sim::shops::parse(&text).map_err(|e| format!("--shops: {e}"))?;
        }
        if let Some(file) = &events {
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("--events: {}: {e}", file.display()))?;
            options.events = car_sim::events::parse(&text).map_err(|e| format!("--events: {e}"))?;
            // The races' routes along the roads of the map the server runs on.
            let terrain = car_sim::terrain::Terrain::load();
            car_sim::events::resolve(&mut options.events, &terrain)
                .map_err(|e| format!("--events: {e}"))?;
        }
        if options.cert.is_some() != options.key.is_some() {
            return Err("--cert and --key go together".into());
        }
        if options.guests && options.database.is_none() {
            return Err("--guests goes with --database (without one, everyone's a guest)".into());
        }
        if options.telegram_token_file.is_some() != options.telegram_chat_file.is_some() {
            return Err("--telegram-token-file and --telegram-chat-file go together".into());
        }
        Ok(options)
    }
}

/// The `user` commands on the database `--database` (in `before` or `after`) names:
/// `after` is the command and its name. The process's exit code.
#[cfg(not(target_arch = "wasm32"))]
fn users(before: &[String], after: &[String]) -> i32 {
    let mut database = None;
    let mut words = Vec::new();
    let mut args = before.iter().chain(after);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--database" => database = args.next().cloned(),
            "--database-file" => match args.next().map(|file| secret(std::path::Path::new(file))) {
                Some(Ok(url)) => database = Some(url),
                Some(Err(e)) => {
                    eprintln!("{e}");
                    return 1;
                }
                None => {
                    eprintln!("--database-file needs a file");
                    return 2;
                }
            },
            other if other.starts_with("--") => {
                eprintln!("user: unknown option {other}");
                return 2;
            }
            _ => {}
        }
    }
    let mut args = after.iter();
    while let Some(arg) = args.next() {
        if arg == "--database" || arg == "--database-file" {
            args.next();
        } else {
            words.push(arg.as_str());
        }
    }
    let Some(url) = database else {
        eprintln!("user: which database? (--database URL)");
        return 2;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let done = runtime.block_on(async {
        let store = car_sim::db::Store::open(&url).await?;
        match words.as_slice() {
            ["add", name] => {
                let name = car_sim::db::clean_name(name)?;
                let password = new_password(&name)?;
                store.add_user(&name, &password).await?;
                println!("Made the account {name}");
            }
            ["password", name] => {
                let password = new_password(name)?;
                store.set_password(name, &password).await?;
                println!("{name} has a new password");
            }
            ["remove", name] => {
                store.remove_user(name).await?;
                println!("Removed the account {name} and everything it had");
            }
            ["list"] => {
                for (name, created) in store.users().await? {
                    let days = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64)
                        - created)
                        / 86_400_000;
                    println!("{name}\t(made {days} days ago)");
                }
            }
            _ => {
                return Err(
                    "user add NAME, user password NAME, user remove NAME or user list".to_string(),
                );
            }
        }
        Ok::<(), String>(())
    });
    match done {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// A new password for `name`: asked twice on the terminal, or a line of standard input.
#[cfg(not(target_arch = "wasm32"))]
fn new_password(name: &str) -> Result<String, String> {
    use std::io::IsTerminal;
    let password = if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password(format!("{name}'s password: "))
            .map_err(|e| e.to_string())?;
        let again = rpassword::prompt_password("Again: ").map_err(|e| e.to_string())?;
        if first != again {
            return Err("The passwords differ".into());
        }
        first
    } else {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        line.trim_end_matches(['\r', '\n']).to_string()
    };
    if password.is_empty() {
        return Err("An empty password isn't one".into());
    }
    Ok(password)
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
