//! Multiplayer: each player simulates their own car and sends its state to a dedicated
//! server, which passes the states on to the players near it, with its traffic's alike;
//! a player shows them all as remote cars (see `client`), and has no traffic of their
//! own (see "Multiplayer" in the notes). `ServerPlugin` and `ClientPlugin` do nothing until there's a `Server` or a
//! `Connection` resource.

pub mod client;
pub mod link;
#[cfg(not(target_arch = "wasm32"))]
pub mod local;
pub mod predict;
pub mod protocol;
#[cfg(not(target_arch = "wasm32"))]
pub mod quic;
pub mod server;
pub mod shadow;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub mod wire;

pub use client::{ChatLine, ClientPlugin, Connection, GhostPlayers, Status, Touching, WantsGhost};
pub use server::{MapPlayers, Presence, Server, ServerPlugin};

use bevy::time::{Fixed, Real, Time};

/// The UDP port servers listen on unless told otherwise.
pub const PORT: u16 = 4433;

/// The WebTransport address of a server given as `host`, `host:port` or a whole
/// `https://` address.
pub fn url(address: &str) -> String {
    let address = address.trim().trim_end_matches('/');
    let host = address.strip_prefix("https://").unwrap_or(address);
    // A port after the host (an IPv6 address is in brackets).
    let has_port = host.rsplit_once(':').is_some_and(|(before, port)| {
        !before.ends_with(':')
            && port.parse::<u16>().is_ok()
            && (!before.contains(':') || before.ends_with(']'))
    });
    if has_port {
        format!("https://{host}")
    } else {
        format!("https://{host}:{PORT}")
    }
}

/// Starts joining the server at `address` (see `url`); the link is there at once and
/// says what went wrong if it fails. Natively, `CAR_SIM_INSECURE=1` accepts any
/// certificate (a development server's throwaway one).
pub fn connect(address: &str) -> Box<dyn link::Link> {
    let url = url(address);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let verify = std::env::var_os("CAR_SIM_INSECURE").is_none();
        Box::new(quic::connect(&url, verify))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(web::connect(&url))
    }
}

/// The app a server runs, 60 updates a second: the world with its traffic (`cars` around
/// each player) and `server` taking players. With `log`, it logs (the dedicated server;
/// in a game the game logs already).
pub fn server_app(server: Server, cars: usize, log: bool) -> bevy::app::App {
    use bevy::app::ScheduleRunnerPlugin;
    use bevy::prelude::*;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    )));
    if log {
        app.add_plugins(bevy::log::LogPlugin::default());
    }
    app.add_plugins((
        crate::WorldPlugin,
        crate::traffic::TrafficPlugin,
        ServerPlugin,
    ))
    .insert_resource(server)
    .insert_resource(crate::traffic::Traffic { cars, ..default() });
    app
}

/// Runs a server's app on this thread until it exits.
pub fn run_server(mut app: bevy::app::App) -> bevy::app::AppExit {
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
    app.run()
}

/// How often a player's car's state is sent (per second).
pub const SEND_RATE: f64 = 30.0;
/// How far other players' cars are seen (m).
pub const NEAR: f32 = 600.0;
/// How often a car's state is passed on to a player within each distance of it (m, per
/// second); the last one reaches to `NEAR`.
pub const RATES: [(f32, f64); 3] = [(150.0, 30.0), (350.0, 10.0), (NEAR, 3.0)];

/// The time on our clock (s) that a car's state, as the last simulation step left it, is
/// from: the step's end, not now (now is up to a step later, unevenly from frame to
/// frame, which would make a steadily moving car seem to jerk).
pub fn stamp(real: &Time<Real>, fixed: &Time<Fixed>) -> f64 {
    real.elapsed_secs_f64() - fixed.overstep().as_secs_f64()
}

/// How fast the clock offset may rise (s per s): enough to follow a sender whose clock
/// runs slow (a few hundred parts per million at most) and a route that got slower for
/// good (20 ms in 4 s), slow enough that it stays near the quickest delivery however many
/// states come in.
const RISE: f64 = 0.005;
/// Our clock less a sender's, as low as it's been seen (the clocks' difference plus the
/// quickest a state came over), rising at most `RISE` per second towards later samples,
/// so a slower route or a slow clock is followed. By time, not per sample: how many
/// states come in (the traffic's share one clock) mustn't lift it off the quickest
/// delivery, or cars would be predicted to a time before their latest state.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClockOffset(Option<(f64, f64)>);

impl ClockOffset {
    /// Takes in a state stamped `then` (their clock) arriving `now` (ours).
    pub(crate) fn heard(&mut self, then: f64, now: f64) {
        let sample = now - then;
        let offset = match self.0 {
            Some((offset, at)) if sample > offset => (offset + RISE * (now - at)).min(sample),
            _ => sample,
        };
        self.0 = Some((offset, now));
    }

    /// The time on their clock that it is `now` on ours, as quickly as states come.
    pub(crate) fn their(&self, now: f64) -> Option<f64> {
        self.0.map(|(offset, _)| now - offset)
    }

    /// The time on our clock that it was `then` on theirs (as quickly as states come).
    pub(crate) fn ours(&self, then: f64) -> Option<f64> {
        self.0.map(|(offset, _)| then + offset)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::*;
    use bevy::time::TimeUpdateStrategy;

    use super::client::{RemoteCar, Tag};
    use super::link::LoopbackListener;
    use super::*;
    use crate::car::{ActiveCar, Car, CarSetup};
    use crate::looks::{Looks, Motion};
    use crate::terrain::{Spawn, Terrain};

    const FRAME: f64 = 1.0 / 60.0;

    fn app() -> App {
        app_on(Terrain::flat())
    }

    fn app_on(terrain: Terrain) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                FRAME,
            )))
            .insert_resource(terrain);
        app
    }

    fn server(listener: &LoopbackListener) -> App {
        let mut app = app();
        app.add_plugins(ServerPlugin)
            .insert_resource(Server::new(listener.clone()));
        app
    }

    /// A player in a car of `chassis` at (x, 0), joined to the server behind `listener`.
    fn player(listener: &LoopbackListener, name: &str, chassis: usize, x: f32) -> App {
        player_on(listener, name, chassis, Vec2::new(x, 0.0), Terrain::flat())
    }

    fn player_on(
        listener: &LoopbackListener,
        name: &str,
        chassis: usize,
        at: Vec2,
        terrain: Terrain,
    ) -> App {
        let car = Car::spawn_at(0, CarSetup::factory(chassis, 0), &terrain, at.x, at.y, 0.0);
        let connection = Connection::new(listener.connect(), name, &terrain);
        let mut app = app_on(terrain);
        app.add_plugins(ClientPlugin)
            .init_resource::<ActiveCar>()
            .insert_resource(connection);
        app.world_mut().spawn((
            Transform::default(),
            Looks::of(&car.setup),
            Motion::of(&car),
            car,
        ));
        app
    }

    fn run(apps: &mut [&mut App], frames: usize) {
        for _ in 0..frames {
            for app in apps.iter_mut() {
                app.update();
            }
        }
    }

    /// The car a player drives.
    fn car(app: &mut App) -> Mut<'_, Car> {
        let entity = (app.world_mut().query_filtered::<Entity, With<Car>>())
            .single(app.world())
            .unwrap();
        app.world_mut().get_mut::<Car>(entity).unwrap()
    }

    /// The cars a player is sent: number, where, their looks and the name they show.
    fn remote(app: &mut App) -> Vec<(u32, Vec3, Option<Looks>, Option<String>)> {
        (app.world_mut()
            .query::<(&RemoteCar, &Transform, Option<&Looks>, Option<&Tag>)>())
        .iter(app.world())
        .map(|(car, transform, looks, tag)| {
            (
                car.id,
                transform.translation,
                looks.cloned(),
                tag.map(|tag| tag.0.clone()),
            )
        })
        .collect()
    }

    fn connection(app: &App) -> &Connection {
        app.world().resource::<Connection>()
    }

    #[test]
    fn addresses_become_urls() {
        assert_eq!(url("cars.example.org"), "https://cars.example.org:4433");
        assert_eq!(
            url(" cars.example.org:5000 "),
            "https://cars.example.org:5000"
        );
        assert_eq!(
            url("https://cars.example.org/"),
            "https://cars.example.org:4433"
        );
        assert_eq!(url("127.0.0.1:47433"), "https://127.0.0.1:47433");
        assert_eq!(url("[::1]:5000"), "https://[::1]:5000");
        assert_eq!(url("[::1]"), "https://[::1]:4433");
    }

    /// The server's copy of a player's car (see `shadow`): how many of the player's checks
    /// it matched to the bit, of how many.
    fn shadow_matched(server: &mut App) -> (usize, usize) {
        let mut peers = server.world_mut().query::<&super::server::Peer>();
        let peer = peers.single(server.world()).unwrap();
        peer.shadow.matched()
    }

    #[test]
    fn the_server_drives_a_players_car_as_they_do() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = driver(&listener, "Anna");
        // Anna drives off, steering this way and that, braking now and then, shifting up.
        for frame in 0..900 {
            let t = frame as f32 * FRAME as f32;
            let mut wheel = anna.world_mut().resource_mut::<crate::input::WheelInput>();
            wheel.throttle = Some(0.5 + 0.5 * (t * 0.8).sin());
            wheel.brake = Some((2.0 * ((t * 1.3).sin() - 0.8)).max(0.0));
            wheel.steer = Some(0.3 * (t * 0.6).sin());
            wheel.shift_up = frame == 200 || frame == 400;
            run(&mut [&mut server, &mut anna], 1);
        }
        let (exact, checks) = shadow_matched(&mut server);
        println!("{exact} of {checks} checks to the bit");
        assert!(checks >= 5, "{checks} checks");
        assert_eq!(exact, checks, "the server's copy went its own way");
        assert!(
            car(&mut anna).fuel.distance > 50.0,
            "Anna should have driven off"
        );
    }

    /// A player whose game drives their car, joined to the server behind `listener`.
    fn driver(listener: &LoopbackListener, name: &str) -> App {
        driver_at(listener, name, Vec2::ZERO)
    }

    /// The same, starting at `at` (map x, z), facing -z.
    fn driver_at(listener: &LoopbackListener, name: &str, at: Vec2) -> App {
        let mut app = player_on(listener, name, 3, at, Terrain::flat());
        app.add_plugins((crate::sim::SimPlugin, crate::car::CarPlugin))
            .init_resource::<crate::input::WheelInput>();
        let engine = car(&mut app).setup.engine.clone();
        car(&mut app).drivetrain.shift_to(&engine, 1);
        app
    }

    #[test]
    fn the_server_is_in_charge_of_a_players_car() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = driver(&listener, "Anna");
        anna.world_mut()
            .resource_mut::<crate::input::WheelInput>()
            .throttle = Some(0.4);
        run(&mut [&mut server, &mut anna], 300);
        let (corrected, compared) = (anna.world())
            .resource::<super::predict::Prediction>()
            .corrections();
        assert!(compared > 10, "{compared} of the server's states compared");
        assert_eq!(corrected, 0, "driven alike, the server agrees");
        // Something only the server knew of shoves her car sideways.
        let mut peers = server.world_mut().query::<&mut super::server::Peer>();
        let mut peer = peers.single_mut(server.world_mut()).unwrap();
        let shoved = peer.shadow.car_mut().expect("the server's car");
        shoved.body.velocity += shoved.body.rotation * Vec3::new(3.0, 0.0, 0.0);
        let corrections = |anna: &App| {
            (anna.world())
                .resource::<super::predict::Prediction>()
                .corrections()
        };
        run(&mut [&mut server, &mut anna], 30);
        let (corrected, compared) = corrections(&anna);
        assert!(corrected > 0, "her game should take up the server's word");
        // From then on they agree again.
        run(&mut [&mut server, &mut anna], 60);
        let (again, more) = corrections(&anna);
        assert_eq!(again, corrected, "corrected again and again");
        assert!(
            more > compared + 3,
            "the server's states should keep coming"
        );
    }

    #[test]
    fn players_cars_meet_on_the_server() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        // Anna stands with the parking brake on; Ben comes from behind, flat out.
        let mut anna = driver(&listener, "Anna");
        let mut ben = driver_at(&listener, "Ben", Vec2::new(0.3, 14.0));
        ben.world_mut()
            .resource_mut::<crate::input::WheelInput>()
            .throttle = Some(1.0);
        let anna_on_server = |server: &mut App| {
            let mut peers = server.world_mut().query::<&super::server::Peer>();
            (peers.iter(server.world()))
                .find(|peer| peer.name.as_deref() == Some("Anna"))
                .and_then(|peer| Some(peer.shadow.car()?.body.position))
        };
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        let before = anna_on_server(&mut server).expect("Anna's car on the server");
        run(&mut [&mut server, &mut anna, &mut ben], 240);
        let after = anna_on_server(&mut server).expect("Anna's car on the server");
        let pushed = before.z - after.z;
        println!("Ben pushed Anna's car {pushed:.2} m on the server");
        assert!(pushed > 0.5, "{pushed} m");
        // Her game took up the server's word.
        let ahead = before.z - car(&mut anna).body.position.z;
        assert!(ahead > 0.5, "her car moved {ahead} m in her game");
    }

    #[test]
    fn players_near_each_other_see_each_other() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 0, 100.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        assert!(matches!(connection(&anna).status, Status::Joined { .. }));
        assert_eq!(
            connection(&anna).players.values().collect::<Vec<_>>(),
            ["Ben"]
        );
        assert_eq!(
            connection(&ben).players.values().collect::<Vec<_>>(),
            ["Anna"]
        );
        // Ben drives off along x at 10 m/s.
        for _ in 0..60 {
            let mut car = car(&mut ben);
            car.body.velocity = Vec3::new(10.0, 0.0, 0.0);
            car.body.position.x += 10.0 * FRAME as f32;
            run(&mut [&mut server, &mut anna, &mut ben], 1);
        }
        let ben_at = car(&mut ben).body.position;
        let seen = remote(&mut anna);
        assert_eq!(seen.len(), 1);
        let (_, at, looks, tag) = &seen[0];
        assert_eq!(looks.as_ref(), Some(&Looks::of(&car(&mut ben).setup)));
        assert_eq!(tag.as_deref(), Some("Ben"), "his car shows his name");
        // Predicted to where he is by now, short only of the frame or two on the way.
        let behind = ben_at.x - at.x;
        println!("Anna sees Ben {behind:.2} m behind where he is");
        assert!((-0.1..0.6).contains(&behind), "{behind}");
        assert_eq!(remote(&mut ben).len(), 1);
    }

    #[test]
    fn a_rescue_is_run_by_the_server_predicted_and_seen_by_the_others() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = driver(&listener, "Anna");
        anna.add_plugins(crate::heli::rescue::RescuePlugin);
        let mut ben = player(&listener, "Ben", 0, 30.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        let heli = |app: &mut App| {
            (app.world_mut().query::<&RemoteCar>())
                .single(app.world())
                .unwrap()
                .latest()
                .and_then(|state| state.heli)
        };
        assert_eq!(heli(&mut ben), None);
        // Anna presses R.
        let press = |anna: &mut App, on: bool| {
            anna.world_mut()
                .resource_mut::<crate::input::WheelInput>()
                .reset = on;
        };
        press(&mut anna, true);
        run(&mut [&mut server, &mut anna, &mut ben], 1);
        press(&mut anna, false);
        run(&mut [&mut server, &mut anna, &mut ben], 120);
        assert!(car(&mut anna).rescue.busy(), "her game should predict it");
        assert!(anna.world().resource::<crate::car::Rescued>().0);
        let mut peers = server.world_mut().query::<&super::server::Peer>();
        let on_server = (peers.iter(server.world()))
            .find(|peer| peer.name.as_deref() == Some("Anna"))
            .and_then(|peer| peer.shadow.car())
            .map(|car| car.rescue.busy());
        assert_eq!(on_server, Some(true), "the server runs it");
        let (corrected, compared) = (anna.world())
            .resource::<super::predict::Prediction>()
            .corrections();
        assert!(compared > 5, "{compared} compared");
        assert_eq!(corrected, 0, "predicted exactly");
        let seen = heli(&mut ben).expect("Ben doesn't see it");
        let ours = car(&mut anna).rescue.pose(0.0).unwrap();
        assert!(seen.position.distance(ours.position) < 5.0);
        // Set down and flown off, still as the server has it.
        run(&mut [&mut server, &mut anna, &mut ben], 1200);
        assert!(!car(&mut anna).rescue.busy(), "never done");
        let (corrected, _) = (anna.world())
            .resource::<super::predict::Prediction>()
            .corrections();
        assert_eq!(corrected, 0, "predicted exactly to the end");
        assert_eq!(heli(&mut ben), None);
    }

    #[test]
    fn everyone_hears_everyones_ping() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 0, 100.0);
        // Every two seconds.
        run(&mut [&mut server, &mut anna, &mut ben], 150);
        for app in [&anna, &ben] {
            let mut pings: Vec<_> = connection(app)
                .pings
                .iter()
                .map(|(&id, &ms)| (id, ms))
                .collect();
            pings.sort();
            assert_eq!(
                pings,
                [(1, 0), (2, 0)],
                "theirs and our own (in memory: none)"
            );
        }
    }

    #[test]
    fn players_far_apart_see_each_other_once_near() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, -480.0);
        let mut ben = player(&listener, "Ben", 3, 480.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        assert_eq!(connection(&anna).players.len(), 1, "knows who's there");
        assert!(remote(&mut anna).is_empty() && remote(&mut ben).is_empty());
        car(&mut ben).body.position.x = -300.0;
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        let seen = remote(&mut anna);
        assert_eq!(seen.len(), 1);
        assert!(seen[0].2.is_some(), "with looks");
        // Far apart again: the proxy goes.
        car(&mut ben).body.position.x = 480.0;
        run(&mut [&mut server, &mut anna, &mut ben], 180);
        assert!(remote(&mut anna).is_empty());
    }

    #[test]
    fn the_server_decides_who_is_on_the_map() {
        // Who Anna has on her map, with Ben 400 m away and then 100 m.
        let on_map = |map: MapPlayers| {
            let listener = LoopbackListener::default();
            let mut server = server(&listener);
            server.world_mut().resource_mut::<Server>().map = map;
            let mut anna = player(&listener, "Anna", 3, 0.0);
            let mut ben = player(&listener, "Ben", 3, 400.0);
            run(&mut [&mut server, &mut anna, &mut ben], 90);
            let now = anna.world().resource::<Time<Real>>().elapsed_secs_f64();
            let far: Vec<_> = connection(&anna).on_map(now).collect();
            car(&mut ben).body.position.x = 100.0;
            run(&mut [&mut server, &mut anna, &mut ben], 90);
            let now = anna.world().resource::<Time<Real>>().elapsed_secs_f64();
            let near: Vec<_> = connection(&anna).on_map(now).collect();
            (far, near)
        };
        let (far, near) = on_map(MapPlayers::Nobody);
        assert!(far.is_empty() && near.is_empty());
        let (far, near) = on_map(MapPlayers::Near(200.0));
        assert!(far.is_empty());
        assert_eq!(near.len(), 1);
        assert!((near[0].position.x - 100.0).abs() < 1.0, "{:?}", near[0]);
        let (far, near) = on_map(MapPlayers::Everyone);
        assert_eq!(far.len(), 1);
        assert!((far[0].position.x - 400.0).abs() < 1.0, "{:?}", far[0]);
        assert_eq!(near.len(), 1);
        // Ben's paint, pointing his way (the factory car faces -z at yaw 0).
        assert_eq!(far[0].paint, CarSetup::factory(3, 0).build.paint);
        assert!(far[0].forward.distance(Vec2::NEG_Y) < 0.01, "{:?}", far[0]);
    }

    #[test]
    fn a_player_on_another_map_is_refused() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut terrain = Terrain::flat();
        terrain.fingerprint = 42;
        let ben = Connection::new(listener.connect(), "Ben", &terrain);
        anna.world_mut().insert_resource(ben);
        run(&mut [&mut server, &mut anna], 10);
        assert_eq!(
            connection(&anna).status,
            Status::Refused("The server has another map".into())
        );
    }

    #[test]
    fn a_stale_web_page_is_told_to_reload() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut terrain = Terrain::flat();
        terrain.fingerprint = 42;
        let ben = Connection::new(listener.connect_from_browser(), "Ben", &terrain);
        anna.world_mut().insert_resource(ben);
        run(&mut [&mut server, &mut anna], 10);
        let Status::Refused(why) = &connection(&anna).status else {
            panic!("not refused");
        };
        assert!(why.starts_with("Press Ctrl+F5"), "{why}");
        assert!(why.ends_with("(The server has another map)"), "{why}");
    }

    #[test]
    fn a_player_who_leaves_is_gone_for_the_others() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 50.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        assert_eq!(remote(&mut anna).len(), 1);
        ben.world_mut().resource_mut::<Connection>().close();
        run(&mut [&mut server, &mut anna], 5);
        assert!(connection(&anna).players.is_empty());
        assert!(remote(&mut anna).is_empty());
    }

    /// A player's chat as "who: what" lines ("· what" for the server's).
    fn chat(app: &App) -> Vec<String> {
        (connection(app).chat.iter())
            .map(|line| match &line.from {
                Some(name) => format!("{name}: {}", line.text),
                None => format!("· {}", line.text),
            })
            .collect()
    }

    #[test]
    fn players_chat_with_everyone() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        run(&mut [&mut server, &mut anna], 10);
        let mut ben = player(&listener, "Ben", 3, 900.0);
        run(&mut [&mut server, &mut anna, &mut ben], 10);
        assert!(connection(&anna).chat_on);
        let anna_writes = |app: &mut App, text: &str| {
            app.world_mut().resource_mut::<Connection>().write(text);
        };
        anna_writes(&mut anna, "  Hello\nBen  ");
        anna_writes(&mut anna, " \t ");
        anna_writes(&mut anna, &"x".repeat(300));
        run(&mut [&mut server, &mut anna, &mut ben], 5);
        let long = format!("Anna: {}", "x".repeat(protocol::CHAT_LENGTH));
        assert_eq!(chat(&anna), ["· Ben joined", "Anna: Hello Ben", &long]);
        assert_eq!(chat(&ben), ["Anna: Hello Ben", &long], "far away too");
        // Too many at once: the rest is held back, with a word to the writer.
        for i in 0..6 {
            anna_writes(&mut anna, &format!("{i}"));
        }
        run(&mut [&mut server, &mut anna, &mut ben], 5);
        assert_eq!(chat(&ben).len(), 2 + 3);
        assert_eq!(
            chat(&anna).last().unwrap(),
            "· Not so fast: wait a few seconds"
        );
        ben.world_mut().resource_mut::<Connection>().close();
        run(&mut [&mut server, &mut anna], 5);
        assert_eq!(chat(&anna).last().unwrap(), "· Ben left");
    }

    #[test]
    fn a_server_without_chat_passes_none_on() {
        let listener = LoopbackListener::default();
        let mut server = app();
        server
            .add_plugins(ServerPlugin)
            .insert_resource(Server::new(listener.clone()).with_chat(false));
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 50.0);
        run(&mut [&mut server, &mut anna, &mut ben], 10);
        assert!(!connection(&anna).chat_on);
        anna.world_mut().resource_mut::<Connection>().write("Hello");
        run(&mut [&mut server, &mut anna, &mut ben], 5);
        assert!(chat(&anna).is_empty() && chat(&ben).is_empty());
    }

    #[test]
    fn a_server_without_name_tags_shows_no_names() {
        let listener = LoopbackListener::default();
        let mut server = app();
        server
            .add_plugins(ServerPlugin)
            .insert_resource(Server::new(listener.clone()).with_name_tags(false));
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 50.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        let seen = remote(&mut anna);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].3, None, "Ben's car shows no name");
        assert_eq!(
            connection(&anna).players.values().collect::<Vec<_>>(),
            ["Ben"],
            "he's still in the players' list"
        );
    }

    #[test]
    fn a_joining_player_is_told_the_shops_events_and_spawn() {
        let listener = LoopbackListener::default();
        let spawn = Spawn::parse("100 -200 90").unwrap();
        let shops = crate::shops::parse("gas 10 20 Pumps\ndyno -5 7 Rollers").unwrap();
        let events = crate::events::parse("hunt 30 40 Hunt").unwrap();
        let mut server = app();
        server.add_plugins(ServerPlugin).insert_resource(
            Server::new(listener.clone())
                .with_shops(shops.clone())
                .with_events(events.clone())
                .with_spawn(spawn),
        );
        let mut anna = player(&listener, "Anna", 3, 0.0);
        assert_eq!(connection(&anna).spawn, Spawn::DEFAULT);
        run(&mut [&mut server, &mut anna], 10);
        assert_eq!(connection(&anna).shops, shops);
        assert_eq!(connection(&anna).events, events);
        assert_eq!(connection(&anna).spawn, spawn);
    }

    #[test]
    fn the_server_tells_who_joined_and_left() {
        #[derive(Resource, Default)]
        struct Told(Vec<(String, bool, usize)>);
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        server.init_resource::<Told>().add_systems(
            Update,
            |mut presence: MessageReader<Presence>, mut told: ResMut<Told>| {
                for p in presence.read() {
                    told.0.push((p.name.clone(), p.joined, p.players));
                }
            },
        );
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 50.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        ben.world_mut().resource_mut::<Connection>().close();
        run(&mut [&mut server, &mut anna], 5);
        let told = &server.world().resource::<Told>().0;
        let mut joined: Vec<_> = told[..2].to_vec();
        joined.sort();
        assert_eq!(joined[0].0, "Anna");
        assert_eq!(joined[1].0, "Ben");
        assert!(joined.iter().all(|(_, joined, _)| *joined));
        assert_eq!(told[2], ("Ben".to_string(), false, 1));
        assert_eq!(told.len(), 3);
    }

    #[test]
    fn cars_that_overlap_each_push_themselves_off() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 30.0);
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        // Ben's car is put right beside Anna's, overlapping it by half a metre (and Anna's
        // game sees it there after the delay).
        let width = 2.0 * Looks::of(&car(&mut anna).setup).hull.half.x;
        car(&mut ben).body.position.x = width - 0.5;
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        // Nothing integrates the cars here: what each got is a push away from the other.
        let (a, b) = (
            car(&mut anna).body.velocity.x,
            car(&mut ben).body.velocity.x,
        );
        println!("Anna pushed off at {a:.3} m/s, Ben at {b:.3} m/s");
        assert!(a < -0.1 && b > 0.1, "both pushed apart");
    }

    /// A car at a shop is a ghost: overlapping cars don't push each other, and it stays a
    /// ghost after the shop until it overlaps nobody.
    #[test]
    fn ghosts_run_into_nothing_until_clear() {
        use crate::looks::Ghost;
        let ghosts = |app: &mut App| {
            (app.world_mut().query_filtered::<(), With<Ghost>>())
                .iter(app.world())
                .count()
        };
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 30.0);
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        anna.insert_resource(WantsGhost(true));
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        let width = 2.0 * Looks::of(&car(&mut anna).setup).hull.half.x;
        car(&mut ben).body.position.x = width - 0.5;
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        assert_eq!(ghosts(&mut anna), 1, "Anna's car is a ghost");
        assert_eq!(ghosts(&mut ben), 1, "and Ben sees it as one");
        let (a, b) = (
            car(&mut anna).body.velocity.x,
            car(&mut ben).body.velocity.x,
        );
        assert!(a == 0.0 && b == 0.0, "nobody pushed: {a}, {b}");
        // Anna leaves the shop: still a ghost while overlapping Ben.
        anna.insert_resource(WantsGhost(false));
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        assert_eq!(ghosts(&mut anna), 1, "still a ghost on top of Ben");
        // Ben drives off.
        car(&mut ben).body.position.x = 30.0;
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        assert_eq!(ghosts(&mut anna), 0, "clear of Ben, no ghost");
        assert_eq!(ghosts(&mut ben), 0);
    }

    /// On a server, the traffic is the server's: a player sees its cars near them where
    /// the server has them, sent like any car (looks, no name), and none of their own.
    #[test]
    fn players_see_the_servers_traffic() {
        use crate::traffic::{TrafficCar, TrafficPlugin};
        let listener = LoopbackListener::default();
        let mut server = app_on(Terrain::baked());
        server
            .add_plugins((ServerPlugin, TrafficPlugin))
            .insert_resource(Server::new(listener.clone()));
        let terrain = Terrain::baked();
        let (spawn, _) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let mut anna = player_on(&listener, "Anna", 3, spawn, terrain);
        run(&mut [&mut server, &mut anna], 120);
        let on_server: Vec<(u32, Vec3, Looks)> = (server.world_mut())
            .query::<(&TrafficCar, &Transform, &Looks)>()
            .iter(server.world())
            .map(|(car, at, looks)| (car.id, at.translation, looks.clone()))
            .collect();
        let seen = remote(&mut anna);
        println!(
            "{} cars on the server, {} seen",
            on_server.len(),
            seen.len()
        );
        assert!(seen.len() >= 10, "{} seen", seen.len());
        let own = (anna.world_mut().query::<&TrafficCar>())
            .iter(anna.world())
            .count();
        assert_eq!(own, 0, "no traffic of her own");
        for (id, at, looks, tag) in &seen {
            let (_, there, factory) = on_server.iter().find(|(i, ..)| i == id).unwrap();
            assert!(at.distance(*there) < 5.0, "car {id} {at} is at {there}");
            assert_eq!(looks.as_ref(), Some(factory));
            assert_eq!(*tag, None);
        }
    }

    /// The server numbers the players' cars and the traffic with one counter, apart from
    /// the players' own numbers.
    #[test]
    fn cars_are_numbered_apart_from_players() {
        use crate::traffic::TrafficPlugin;
        let listener = LoopbackListener::default();
        let mut server = app_on(Terrain::baked());
        server
            .add_plugins((ServerPlugin, TrafficPlugin))
            .insert_resource(Server::new(listener.clone()));
        let terrain = Terrain::baked();
        let (spawn, _) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let mut anna = player_on(&listener, "Anna", 3, spawn, terrain);
        run(&mut [&mut server, &mut anna], 60);
        // Ben joins with traffic already numbered.
        let mut ben = player_on(
            &listener,
            "Ben",
            0,
            spawn + Vec2::new(0.0, 20.0),
            Terrain::baked(),
        );
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        let ben_id = connection(&anna)
            .players
            .iter()
            .find(|(_, name)| *name == "Ben")
            .map(|(&id, _)| id)
            .unwrap();
        let seen = remote(&mut anna);
        let (bens_car, ..) = seen
            .iter()
            .find(|(.., tag)| tag.as_deref() == Some("Ben"))
            .unwrap();
        assert_ne!(*bens_car, ben_id, "his car's number isn't his");
        let mut ids: Vec<u32> = seen.iter().map(|(id, ..)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), seen.len());
    }

    /// Puts the player called `name` into `realm` on the server.
    fn move_to_realm(server: &mut App, name: &str, realm: crate::traffic::Realm) {
        let entity = (server.world_mut().query::<(Entity, &server::Peer)>())
            .iter(server.world())
            .find(|(_, peer)| peer.name.as_deref() == Some(name))
            .map(|(entity, _)| entity)
            .unwrap();
        server.world_mut().entity_mut(entity).insert(realm);
    }

    /// Players in another realm see neither the others nor their traffic, nor their
    /// chat; a realm without traffic has none.
    #[test]
    fn realms_see_only_themselves() {
        use crate::traffic::{Realm, Traffic, TrafficCar, TrafficPlugin};
        let listener = LoopbackListener::default();
        let mut server = app_on(Terrain::baked());
        server
            .add_plugins((ServerPlugin, TrafficPlugin))
            .insert_resource(Server::new(listener.clone()));
        let terrain = Terrain::baked();
        let (spawn, _) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let mut anna = player_on(&listener, "Anna", 3, spawn, terrain);
        let mut ben = player_on(
            &listener,
            "Ben",
            0,
            spawn + Vec2::new(0.0, 20.0),
            Terrain::baked(),
        );
        let mut cleo = player_on(
            &listener,
            "Cleo",
            0,
            spawn + Vec2::new(0.0, -20.0),
            Terrain::baked(),
        );
        run(&mut [&mut server, &mut anna, &mut ben, &mut cleo], 20);
        let hunt = Realm(7);
        (server.world_mut().resource_mut::<Traffic>().realms).insert(hunt, 0);
        move_to_realm(&mut server, "Ben", hunt);
        move_to_realm(&mut server, "Cleo", hunt);
        run(&mut [&mut server, &mut anna, &mut ben, &mut cleo], 120);
        let tags = |app: &mut App| -> Vec<Option<String>> {
            remote(app).into_iter().map(|(.., tag)| tag).collect()
        };
        let annas = tags(&mut anna);
        assert!(annas.len() >= 10, "her traffic: {annas:?}");
        assert!(
            annas.iter().all(Option::is_none),
            "no Ben or Cleo: {annas:?}"
        );
        assert_eq!(tags(&mut ben), [Some("Cleo".to_string())], "only Cleo");
        assert_eq!(tags(&mut cleo), [Some("Ben".to_string())], "only Ben");
        let realms: Vec<Realm> = (server.world_mut())
            .query::<(&TrafficCar, Option<&Realm>)>()
            .iter(server.world())
            .map(|(_, realm)| Realm::of(realm))
            .collect();
        assert!(realms.iter().all(|&realm| realm == Realm::NORMAL));
        // With traffic in the realm, it gets its own.
        (server.world_mut().resource_mut::<Traffic>().realms).insert(hunt, 5);
        run(&mut [&mut server, &mut anna, &mut ben, &mut cleo], 60);
        let in_hunt = (server.world_mut())
            .query::<(&TrafficCar, Option<&Realm>)>()
            .iter(server.world())
            .filter(|(_, realm)| Realm::of(*realm) == hunt)
            .count();
        assert_eq!(in_hunt, 5);
        assert_eq!(tags(&mut ben).len(), 6, "Cleo and the hunt's traffic");
        assert_eq!(tags(&mut anna).len(), annas.len(), "none of it for Anna");
        // The chat stays in its realm.
        (ben.world_mut().resource_mut::<Connection>()).write("Hi Cleo");
        (anna.world_mut().resource_mut::<Connection>()).write("Anyone?");
        run(&mut [&mut server, &mut anna, &mut ben, &mut cleo], 5);
        let said = |app: &App| -> Vec<String> {
            (chat(app).into_iter())
                .filter(|line| !line.starts_with('·'))
                .collect()
        };
        assert_eq!(said(&cleo), ["Ben: Hi Cleo"]);
        assert_eq!(said(&anna), ["Anna: Anyone?"]);
    }

    /// The round a player is in, as they were told.
    fn round(app: &App) -> Option<crate::events::round::Round> {
        connection(app).round.clone()
    }

    fn act(app: &mut App, action: crate::events::round::Action) {
        app.world_mut().resource_mut::<Connection>().act(action);
    }

    /// A car hunt: into the lobby on the marker, the hunted's part handed over, the
    /// settings, the start; the round in a realm of its own; the hunted caught standing
    /// still; everyone back in the lobby, told how it ended.
    #[test]
    fn a_car_hunt_runs_in_a_realm_of_its_own() {
        use crate::events::hunt::{Ending, Settings};
        use crate::events::round::{Action, Game, Outcome, Phase};
        let listener = LoopbackListener::default();
        let events = crate::events::parse("hunt 0 0 Hunt").unwrap();
        let mut server = server(&listener);
        (server.world_mut().resource_mut::<Server>()).set_events(events);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 6.0);
        let mut cleo = player(&listener, "Cleo", 3, 40.0);
        let mut all = [&mut server, &mut anna, &mut ben, &mut cleo];
        run(&mut all, 20);
        let id = |app: &App| match connection(app).status {
            Status::Joined { you } => you,
            _ => panic!("not joined"),
        };
        let (anna_id, ben_id) = (id(all[1]), id(all[2]));
        // Too far from the marker: nothing.
        act(all[3], Action::Enter(0));
        act(all[1], Action::Enter(0));
        run(&mut all, 5);
        act(all[2], Action::Enter(0));
        run(&mut all, 5);
        assert_eq!(round(all[3]), None, "Cleo is too far away");
        let lobby = round(all[2]).expect("Ben is in the lobby");
        assert_eq!(lobby.players, [anna_id, ben_id]);
        assert_eq!(lobby.host, anna_id, "the first in is hunted");
        assert_eq!(connection(all[3]).busy.len(), 2, "Cleo sees who's in it");
        // Only the hunted sets things and starts.
        act(all[2], Action::Start);
        act(all[1], Action::Host(ben_id));
        let settings = Settings {
            head_start: 2,
            traffic: false,
            ..Settings::default()
        };
        run(&mut all, 5);
        act(all[1], Action::Game(Game::Hunt(settings)));
        run(&mut all, 5);
        let lobby = round(all[1]).unwrap();
        assert_eq!(lobby.host, ben_id);
        assert!(!lobby.running());
        assert_eq!(
            lobby.game,
            Game::Hunt(Settings::default()),
            "Anna isn't hunted any more"
        );
        act(all[2], Action::Game(Game::Hunt(settings)));
        act(all[2], Action::Start);
        run(&mut all, 30);
        let running = round(all[1]).unwrap();
        assert!(running.running());
        assert_eq!(running.game, Game::Hunt(settings));
        // In a realm of their own: Cleo, 40 m away, sees neither; they see each other
        // (named, and on each other's map).
        let tags = |app: &mut App| -> Vec<Option<String>> {
            remote(app).into_iter().map(|(.., tag)| tag).collect()
        };
        assert_eq!(tags(all[3]), []);
        assert_eq!(tags(all[1]), [Some("Ben".to_string())]);
        // (Cleo, from before, fades from the map after 3 s.)
        run(&mut all, 60 * 4);
        let now = all[1].world().resource::<Time<Real>>().elapsed_secs_f64();
        assert_eq!(connection(all[1]).on_map(now).count(), 1);
        // A helicopter holds over the hunted, sent to everyone in the round alone.
        let ben_at = all[2]
            .world_mut()
            .query::<&Car>()
            .single(all[2].world())
            .unwrap();
        let ben_at = ben_at.body.position;
        for i in [1, 2] {
            let (heli, heard) = connection(all[i]).hunt_heli.expect("no helicopter");
            assert!(now - heard < 0.2, "not sent lately");
            let above = heli.position - ben_at;
            assert!(above.xz().length() < 10.0, "{above}");
            assert!((above.y - crate::heli::CHASE_HEIGHT).abs() < 5.0, "{above}");
        }
        assert!(connection(all[3]).hunt_heli.is_none());
        // Ben stands still: caught 10 s after the head start.
        run(&mut all, 60 * 5);
        assert!(round(all[2]).unwrap().running(), "not yet");
        run(&mut all, 60 * 4);
        let after = round(all[2]).unwrap();
        let Phase::Lobby {
            last: Some(Outcome::Hunt(outcome)),
        } = &after.phase
        else {
            panic!("still {:?}", after.phase);
        };
        assert_eq!(outcome.hunted, "Ben");
        let Ending::Caught(time) = outcome.ending else {
            panic!("{outcome:?}");
        };
        assert!((9.5..11.0).contains(&time), "{time}");
        assert_eq!(after.players, [anna_id, ben_id], "both in the lobby");
        run(&mut all, 60);
        assert_eq!(tags(all[3]).len(), 2, "back in Cleo's world");
        // The hunted leaving ends a round under way.
        act(all[2], Action::Start);
        run(&mut all, 10);
        assert!(round(all[1]).unwrap().running());
        act(all[2], Action::Leave);
        run(&mut all, 10);
        assert_eq!(round(all[2]), None);
        let after = round(all[1]).unwrap();
        assert_eq!(after.players, [anna_id]);
        assert_eq!(after.host, anna_id);
        let Phase::Lobby {
            last: Some(Outcome::Hunt(outcome)),
        } = &after.phase
        else {
            panic!("still {:?}", after.phase);
        };
        assert_eq!(outcome.ending, Ending::HuntedLeft);
        assert_eq!(connection(all[3]).busy.len(), 1);
    }

    /// A server with a car hunt at the origin.
    fn hunt_server(listener: &LoopbackListener) -> App {
        let events = crate::events::parse("hunt 0 0 Hunt").unwrap();
        let mut server = server(listener);
        (server.world_mut().resource_mut::<Server>()).set_events(events);
        server
    }

    /// A player's game with the events' lobby, the car hunt at the origin on its map.
    fn hunter(listener: &LoopbackListener, name: &str, x: f32) -> App {
        use crate::car::{Held, ResetRule};
        use crate::events::{Events, Placed, lobby::LobbyPlugin};
        let event = crate::events::parse("hunt 0 0 Hunt").unwrap().remove(0);
        let mut app = player(listener, name, 3, x);
        app.add_plugins(LobbyPlugin)
            .init_resource::<Held>()
            .init_resource::<ResetRule>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Events {
                list: vec![Placed {
                    event,
                    marker: Vec2::ZERO,
                }],
            });
        app
    }

    /// Tagging: a hunter's car touching the hunted's ends the round at once.
    #[test]
    fn a_hunter_tags_the_hunted() {
        use crate::events::hunt::{Catch, Ending, Settings};
        use crate::events::round::{Action, Game, Outcome, Phase};
        let listener = LoopbackListener::default();
        let mut server = hunt_server(&listener);
        let mut anna = hunter(&listener, "Anna", 0.0);
        let mut ben = hunter(&listener, "Ben", 3.0);
        let mut all = [&mut server, &mut anna, &mut ben];
        run(&mut all, 30);
        let lobby = round(all[1]).expect("in the lobby by stopping on the marker");
        let you = |app: &App| match connection(app).status {
            Status::Joined { you } => you,
            _ => panic!("not joined"),
        };
        let (h, k) = if you(all[1]) == lobby.host {
            (1, 2)
        } else {
            (2, 1)
        };
        let settings = Settings {
            head_start: 0,
            traffic: false,
            catch: Catch::Tag,
            ..Settings::default()
        };
        act(all[h], Action::Game(Game::Hunt(settings)));
        act(all[h], Action::Start);
        run(&mut all, 10);
        assert!(round(all[k]).unwrap().running());
        // Apart first (they waited as ghosts, which the shops' plugin, not here, would stop
        // wanting), then the hunter's car into the hunted's.
        for i in [h, k] {
            all[i].world_mut().resource_mut::<WantsGhost>().0 = false;
        }
        car(all[k]).body.position.x += 30.0;
        run(&mut all, 30);
        assert!(round(all[k]).unwrap().running(), "nobody touched yet");
        let at = car(all[h]).body.position;
        car(all[k]).body.position = at + Vec3::X * 1.5;
        run(&mut all, 20);
        let after = round(all[h]).unwrap();
        let Phase::Lobby {
            last: Some(Outcome::Hunt(outcome)),
        } = &after.phase
        else {
            panic!("still {:?}", after.phase);
        };
        let hunter_name = if k == 1 { "Anna" } else { "Ben" };
        let Ending::Tagged { by, after } = &outcome.ending else {
            panic!("{outcome:?}");
        };
        assert_eq!(by, hunter_name);
        assert!(*after < 2.0, "{after}");
    }

    /// A player's game in a car hunt: stopping on the marker goes in, the round lends
    /// the cars it says, holds the hunters for the head start and sets what R does; at
    /// the end everyone has their own car back, as it was.
    #[test]
    fn a_hunt_lends_cars_and_gives_them_back() {
        use crate::car::{Held, ResetRule};
        use crate::chassis::CHASSIS;
        use crate::events::hunt::{Cars, Settings};
        use crate::events::round::{Action, Game};
        let listener = LoopbackListener::default();
        let mut server = hunt_server(&listener);
        let mut anna = hunter(&listener, "Anna", 0.0);
        let mut ben = hunter(&listener, "Ben", 3.0);
        car(&mut anna).refuel(0.5);
        let annas_fuel = car(&mut anna).fuel.litres();
        let mut all = [&mut server, &mut anna, &mut ben];
        run(&mut all, 30);
        let lobby = round(all[1]).expect("in the lobby by stopping on the marker");
        assert_eq!(lobby.players.len(), 2);
        // Whoever came in first is hunted.
        let you = |app: &App| match connection(app).status {
            Status::Joined { you } => you,
            _ => panic!("not joined"),
        };
        let (h, k) = if you(all[1]) == lobby.host {
            (1, 2)
        } else {
            (2, 1)
        };
        let settings = Settings {
            head_start: 2,
            hunters_reset: false,
            cars: Cars::MuscleVsRally,
            traffic: false,
            ..Settings::default()
        };
        act(all[h], Action::Game(Game::Hunt(settings)));
        act(all[h], Action::Start);
        run(&mut all, 10);
        let chassis = |app: &mut App| CHASSIS[car(app).setup.build.chassis].name;
        assert_eq!(chassis(all[h]), "Skarn", "the hunted's rally car");
        assert_eq!(chassis(all[k]), "Harrow", "the hunter's muscle car");
        let parts = car(all[k]).setup.engine.parts.clone();
        let family = &crate::physics::engine::families::FAMILIES[parts.family];
        assert_eq!(family.versions[parts.version].code, "M50", "the smaller V8");
        assert!(
            all[k].world().resource::<Held>().0,
            "held for the head start"
        );
        assert!(!all[h].world().resource::<Held>().0);
        assert_eq!(*all[h].world().resource::<ResetRule>(), ResetRule::Never);
        assert_eq!(
            *all[k].world().resource::<ResetRule>(),
            ResetRule::Never,
            "hunters can't reset"
        );
        run(&mut all, 150);
        assert!(!all[k].world().resource::<Held>().0, "off they go");
        // The hunted stands still and is caught; both have their own cars back.
        run(&mut all, 60 * 11);
        assert!(!round(all[h]).unwrap().running());
        assert_eq!(chassis(all[h]), "Tarn");
        assert_eq!(chassis(all[k]), "Tarn");
        assert!(
            (car(all[1]).fuel.litres() - annas_fuel).abs() < 0.01,
            "Anna's own"
        );
        assert_eq!(*all[h].world().resource::<ResetRule>(), ResetRule::Road);
    }

    /// A race event at the origin, with a square loop 400 m a side around it: its
    /// checkpoints in the middle of each side (the first on the north side, going east),
    /// two grid spots just before the first.
    fn race_event() -> crate::events::Event {
        use crate::events::race::{GridSpot, Route};
        let line = vec![
            Vec2::new(0.0, -200.0),
            Vec2::new(200.0, -200.0),
            Vec2::new(200.0, 0.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(0.0, 200.0),
            Vec2::new(-200.0, 200.0),
            Vec2::new(-200.0, 0.0),
            Vec2::new(-200.0, -200.0),
            Vec2::new(0.0, -200.0),
        ];
        let spot = |x: f32, z: f32| GridSpot {
            at: Vec2::new(x, z),
            heading: 90.0,
        };
        crate::events::Event {
            kind: crate::events::EventKind::Race,
            name: "Races".into(),
            at: Vec2::ZERO,
            routes: vec![Route {
                name: "Square".into(),
                looped: true,
                grid: vec![spot(-10.0, -198.5), spot(-16.0, -201.5)],
                checkpoints: vec![line[0], line[2], line[4], line[6]],
                widths: vec![6.0; line.len()],
                marks: vec![0, 2, 4, 6, 8],
                line,
            }],
        }
    }

    fn race_server(listener: &LoopbackListener) -> App {
        let mut server = server(listener);
        (server.world_mut().resource_mut::<Server>()).set_events(vec![race_event()]);
        server
    }

    /// A player's game with the events' lobby and the race at the origin.
    fn racer(listener: &LoopbackListener, name: &str, x: f32) -> App {
        use crate::car::{Held, ResetRule};
        use crate::events::{Events, Placed, lobby::LobbyPlugin};
        let mut app = player(listener, name, 3, x);
        app.add_plugins(LobbyPlugin)
            .init_resource::<Held>()
            .init_resource::<ResetRule>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(Events {
                list: vec![Placed {
                    event: race_event(),
                    marker: Vec2::ZERO,
                }],
            });
        app
    }

    /// Drives a player's car (by hand) along the course from `from` to `to` metres, a few
    /// frames every 8 m, everyone's games running.
    fn drive_along(
        all: &mut [&mut App],
        who: usize,
        course: &crate::events::race::Course,
        from: f32,
        to: f32,
    ) {
        let mut s = from;
        while s < to {
            s = (s + 8.0).min(to);
            let (p, forward, _) = course.point(s % course.length());
            let mut car = car(all[who]);
            car.body.position = Vec3::new(p.x, car.body.position.y, p.y);
            car.body.velocity = Vec3::new(forward.x, 0.0, forward.y) * 20.0;
            run(all, 3);
        }
    }

    /// A race: into the lobby, the host's settings and start; everyone on the grid, held
    /// for the countdown, the others ghosts without contact, R to the last gate; the gates
    /// in order (one cut doesn't count), the finish; the results once the last racer is
    /// through or gone.
    #[test]
    fn a_race_runs_from_the_grid_to_the_results() {
        use crate::car::{Held, ResetRule};
        use crate::events::race::{COUNTDOWN, Course, Settings};
        use crate::events::round::{Action, Game, Outcome, Phase};
        let listener = LoopbackListener::default();
        let mut server = race_server(&listener);
        let mut anna = racer(&listener, "Anna", 0.0);
        let mut ben = racer(&listener, "Ben", 3.0);
        let mut all = [&mut server, &mut anna, &mut ben];
        run(&mut all, 20);
        // Stopped on the marker, asked what to do: race.
        assert!(
            all[1]
                .world()
                .resource::<crate::events::lobby::EventState>()
                .choosing
                .is_some()
        );
        act(all[1], Action::Enter(0));
        run(&mut all, 5);
        act(all[2], Action::Enter(0));
        run(&mut all, 5);
        let lobby = round(all[2]).expect("Ben is in the lobby");
        assert_eq!(lobby.players.len(), 2);
        assert!(matches!(lobby.game, Game::Race(_)));
        let settings = Settings {
            laps: 1,
            collisions: false,
            ..Settings::default()
        };
        act(all[2], Action::Start);
        act(all[1], Action::Game(Game::Race(settings)));
        act(all[1], Action::Start);
        run(&mut all, 10);
        let running = round(all[2]).unwrap();
        assert!(running.running(), "Anna, the host, started it");
        assert_eq!(running.game, Game::Race(settings));
        // On the grid, held, the others ghosts, R to the grid.
        let course = Course::new(&race_event().routes[0], 1);
        for (i, slot) in [(1, 0), (2, 1)] {
            let p = car(all[i]).body.position.xz();
            let (spot, _) = course.grid(slot);
            assert!(p.distance(spot) < 0.5, "{p} isn't at {spot}");
            assert!(
                all[i].world().resource::<Held>().0,
                "held for the countdown"
            );
            assert!(!car(all[i]).parking_brake, "free to go at the green light");
            assert!(all[i].world().resource::<GhostPlayers>().0);
            assert!(matches!(
                *all[i].world().resource::<ResetRule>(),
                ResetRule::Spot(..)
            ));
        }
        run(&mut all, (COUNTDOWN / FRAME) as usize + 10);
        assert!(!all[1].world().resource::<Held>().0, "go");
        // Anna drives through the first gates, then cuts across past the next one: it
        // doesn't count until she goes back through it.
        let gates = &course.gates;
        drive_along(&mut all, 1, &course, 20.0, gates[1].s + 5.0);
        let passed = |app: &App| round(app).unwrap().standing().unwrap().racers[0].gates;
        assert_eq!(passed(all[2]), 2);
        drive_along(&mut all, 1, &course, gates[2].s + 20.0, gates[3].s + 5.0);
        assert_eq!(passed(all[2]), 2, "gate 3 was skipped");
        drive_along(
            &mut all,
            1,
            &course,
            gates[2].s - 10.0,
            course.length() + 5.0,
        );
        let standing = round(all[2]).unwrap().standing().unwrap().clone();
        let anna = &standing.racers[0];
        assert_eq!(usize::from(anna.gates), gates.len());
        assert!(anna.finished.is_some(), "through");
        assert_eq!(anna.laps.len(), 1);
        assert!(standing.closes.is_some(), "the others have a minute");
        // Ben leaves: the race is over, Anna first.
        act(all[2], Action::Leave);
        run(&mut all, 10);
        assert_eq!(round(all[2]), None);
        let after = round(all[1]).unwrap();
        let Phase::Lobby {
            last: Some(Outcome::Race(results)),
        } = &after.phase
        else {
            panic!("still {:?}", after.phase);
        };
        assert_eq!(results.entries[0].name, "Anna");
        assert!(results.entries[0].time.is_some());
        assert_eq!(results.entries[1].name, "Ben");
        assert!(results.entries[1].left);
        assert!(
            after.best.is_empty(),
            "races don't make the time trials' list"
        );
        assert_eq!(*all[1].world().resource::<ResetRule>(), ResetRule::Road);
        assert!(!all[1].world().resource::<GhostPlayers>().0);
    }

    /// A time trial: a round of one nobody else joins; started again, then a lap that
    /// goes on the best times.
    #[test]
    fn a_time_trial_puts_its_lap_on_the_best_times() {
        use crate::car::{Held, ResetRule};
        use crate::events::race::{COUNTDOWN, Course, Settings};
        use crate::events::round::{Action, Game, Outcome, Phase};
        let listener = LoopbackListener::default();
        let mut server = race_server(&listener);
        let mut anna = racer(&listener, "Anna", 0.0);
        let mut ben = racer(&listener, "Ben", 3.0);
        let mut all = [&mut server, &mut anna, &mut ben];
        run(&mut all, 20);
        act(all[1], Action::Trial(0));
        run(&mut all, 5);
        act(all[2], Action::Enter(0));
        run(&mut all, 5);
        assert_eq!(
            round(all[1]).unwrap().players.len(),
            1,
            "a trial is private"
        );
        assert_eq!(
            round(all[2]).unwrap().players.len(),
            1,
            "Ben opened a race lobby"
        );
        let settings = Settings {
            laps: 1,
            ..Settings::default()
        };
        act(all[1], Action::Game(Game::Race(settings)));
        act(all[1], Action::Start);
        run(&mut all, 10);
        let trial = round(all[1]).unwrap();
        assert!(trial.running(), "alone");
        assert_eq!(trial.game, Game::Trial(settings), "still a trial");
        assert_eq!(*all[1].world().resource::<ResetRule>(), ResetRule::Never);
        run(&mut all, (COUNTDOWN / FRAME) as usize + 10);
        let course = Course::new(&race_event().routes[0], 1);
        drive_along(&mut all, 1, &course, 20.0, course.gates[2].s + 5.0);
        // Start again: back on the grid, held, nothing passed.
        act(all[1], Action::Restart);
        run(&mut all, 10);
        let p = car(all[1]).body.position.xz();
        assert!(p.distance(course.grid(0).0) < 0.5);
        assert!(all[1].world().resource::<Held>().0);
        assert_eq!(
            round(all[1]).unwrap().standing().unwrap().racers[0].gates,
            0
        );
        run(&mut all, (COUNTDOWN / FRAME) as usize + 10);
        // (To just past the finish: moving the car after the trial ends would leave its
        // lobby.)
        drive_along(&mut all, 1, &course, 20.0, course.length() + 3.0);
        run(&mut all, 5);
        let after = round(all[1]).unwrap();
        let Phase::Lobby {
            last: Some(Outcome::Race(results)),
        } = &after.phase
        else {
            panic!("still {:?}", after.phase);
        };
        let time = results.entries[0].time.unwrap();
        assert_eq!(after.best.len(), 1);
        assert_eq!(after.best[0].name, "Anna");
        assert!((after.best[0].time - time).abs() < 0.01);
        // Ben's lobby, on the same route, sees it too.
        assert_eq!(round(all[2]).unwrap().best, after.best);
    }

    /// A car the server's catalogues don't have (a newer game's) gets a clear refusal.
    #[test]
    fn a_car_the_server_doesnt_know_is_refused_with_a_reason() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let entity = (anna.world_mut().query_filtered::<Entity, With<Car>>())
            .single(anna.world())
            .unwrap();
        anna.world_mut()
            .get_mut::<Looks>(entity)
            .unwrap()
            .build
            .chassis = crate::chassis::CHASSIS.len();
        run(&mut [&mut server, &mut anna], 10);
        let Status::Refused(why) = &connection(&anna).status else {
            panic!("{:?}", connection(&anna).status);
        };
        assert!(why.contains("doesn't know your car"), "{why}");
    }

    /// A link that hands over what came in with the datagrams first (states and traffic
    /// overtaking the stream, as they may on the internet).
    struct Overtaking {
        link: link::Loopback,
        held: std::collections::VecDeque<Vec<u8>>,
    }

    impl link::Link for Overtaking {
        fn send(&mut self, reliable: bool, message: Vec<u8>) {
            self.link.send(reliable, message);
        }
        fn receive(&mut self) -> Option<Vec<u8>> {
            while let Some(message) = self.link.receive() {
                self.held.push_back(message);
            }
            let datagram = self.held.iter().position(|m| {
                let message = wire::decode::<protocol::ToClient>(m);
                message.is_some_and(|m| !m.reliable())
            });
            match datagram {
                Some(i) => self.held.remove(i),
                None => self.held.pop_front(),
            }
        }
        fn is_open(&self) -> bool {
            self.link.is_open()
        }
        fn close(&mut self) {
            self.link.close();
        }
    }

    /// A car's first state arriving before its looks: the car still gets them, and its
    /// name.
    #[test]
    fn a_state_overtaking_the_looks_still_shows_the_car() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let link = Overtaking {
            link: listener.connect(),
            held: Default::default(),
        };
        let mut ben = app();
        let terrain = Terrain::flat();
        let car = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 30.0, 0.0, 0.0);
        ben.add_plugins(ClientPlugin)
            .init_resource::<ActiveCar>()
            .insert_resource(Connection::new(link, "Ben", &terrain));
        ben.world_mut().spawn((
            Transform::default(),
            Looks::of(&car.setup),
            Motion::of(&car),
            car,
        ));
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        let seen = remote(&mut ben);
        assert_eq!(seen.len(), 1);
        assert!(seen[0].2.is_some(), "Anna's car has its looks");
        assert_eq!(seen[0].3.as_deref(), Some("Anna"));
    }
}
