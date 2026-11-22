//! Multiplayer: each player simulates their own car and sends its state to a dedicated
//! server, which passes the states on to the players near it, with its traffic's alike;
//! a player shows them all as remote cars (see `client`), and has no traffic of their
//! own (see "Multiplayer" in the notes). `ServerPlugin` and `ClientPlugin` do nothing until there's a `Server` or a
//! `Connection` resource.

pub mod client;
pub mod link;
pub mod protocol;
#[cfg(not(target_arch = "wasm32"))]
pub mod quic;
pub mod server;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub mod wire;

pub use client::{ChatLine, ClientPlugin, Connection, Status, WantsGhost};
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
    use crate::terrain::Terrain;

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
    fn a_joining_player_is_told_the_shops() {
        let listener = LoopbackListener::default();
        let shops = crate::shops::parse("gas 10 20 Pumps\ndyno -5 7 Rollers").unwrap();
        let mut server = app();
        server
            .add_plugins(ServerPlugin)
            .insert_resource(Server::new(listener.clone()).with_shops(shops.clone()));
        let mut anna = player(&listener, "Anna", 3, 0.0);
        run(&mut [&mut server, &mut anna], 10);
        assert_eq!(connection(&anna).shops, shops);
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
        let (spawn, _) = terrain.road_spawns(1, 0.0)[0];
        let mut anna = player_on(&listener, "Anna", 3, spawn, terrain);
        anna.add_plugins(TrafficPlugin);
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
        let (spawn, _) = terrain.road_spawns(1, 0.0)[0];
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
        let (spawn, _) = terrain.road_spawns(1, 0.0)[0];
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
