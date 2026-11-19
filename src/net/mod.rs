//! Multiplayer: each player simulates their own car and sends its state to a dedicated
//! server, which passes the states on to the players near it; other players' cars are
//! shown as proxies (see `client`). Traffic is the server's (see "Multiplayer" in the
//! notes). `ServerPlugin` and `ClientPlugin` do nothing until there's a `Server` or a
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

pub use client::{ClientPlugin, Connection, Status};
pub use server::{Server, ServerPlugin};

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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::*;
    use bevy::time::TimeUpdateStrategy;

    use super::client::Proxy;
    use super::link::LoopbackListener;
    use super::*;
    use crate::car::{ActiveCar, Car, CarSetup};
    use crate::looks::{Looks, Motion};
    use crate::terrain::Terrain;

    const FRAME: f64 = 1.0 / 60.0;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                FRAME,
            )))
            .insert_resource(Terrain::flat());
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
        let mut app = app();
        let terrain = Terrain::flat();
        let car = Car::spawn_at(0, CarSetup::factory(chassis, 0), &terrain, x, 0.0, 0.0);
        let connection = Connection::new(listener.connect(), name, &terrain);
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

    /// The other players' cars a player sees: number, where, and their looks.
    fn proxies(app: &mut App) -> Vec<(u32, Vec3, Option<Looks>)> {
        (app.world_mut()
            .query::<(&Proxy, &Transform, Option<&Looks>)>())
        .iter(app.world())
        .map(|(proxy, transform, looks)| (proxy.id, transform.translation, looks.cloned()))
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
        let seen = proxies(&mut anna);
        assert_eq!(seen.len(), 1);
        let (_, at, looks) = &seen[0];
        assert_eq!(looks.as_ref(), Some(&Looks::of(&car(&mut ben).setup)));
        // Shown a little in the past: the delay and a frame or two on the way.
        let behind = ben_at.x - at.x;
        println!("Anna sees Ben {behind:.2} m behind where he is");
        assert!((0.8..2.0).contains(&behind), "{behind}");
        assert_eq!(proxies(&mut ben).len(), 1);
    }

    #[test]
    fn players_far_apart_see_each_other_once_near() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, -480.0);
        let mut ben = player(&listener, "Ben", 3, 480.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        assert_eq!(connection(&anna).players.len(), 1, "knows who's there");
        assert!(proxies(&mut anna).is_empty() && proxies(&mut ben).is_empty());
        car(&mut ben).body.position.x = -300.0;
        run(&mut [&mut server, &mut anna, &mut ben], 30);
        let seen = proxies(&mut anna);
        assert_eq!(seen.len(), 1);
        assert!(seen[0].2.is_some(), "with looks");
        // Far apart again: the proxy goes.
        car(&mut ben).body.position.x = 480.0;
        run(&mut [&mut server, &mut anna, &mut ben], 180);
        assert!(proxies(&mut anna).is_empty());
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
    fn a_player_who_leaves_is_gone_for_the_others() {
        let listener = LoopbackListener::default();
        let mut server = server(&listener);
        let mut anna = player(&listener, "Anna", 3, 0.0);
        let mut ben = player(&listener, "Ben", 3, 50.0);
        run(&mut [&mut server, &mut anna, &mut ben], 60);
        assert_eq!(proxies(&mut anna).len(), 1);
        ben.world_mut().resource_mut::<Connection>().close();
        run(&mut [&mut server, &mut anna], 5);
        assert!(connection(&anna).players.is_empty());
        assert!(proxies(&mut anna).is_empty());
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
}
