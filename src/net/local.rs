//! Playing alone (native only): a server of the game's own, on a thread of its own, joined
//! in memory like any server. It has the repo's shops and events and the default traffic,
//! so playing alone is playing online with nobody else there. What the player has is kept
//! in an SQLite file in the game's data folder (`alone.db`, as the default user). It stops
//! when its link is dropped (the `Connection` goes), saving the player's car first.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::app::AppExit;
use bevy::prelude::*;

use super::accounts::Accounts;
use super::link::{Link, Loopback, LoopbackListener};
use super::server::Peer;
use super::{MapPlayers, Server, run_server, server_app};
use crate::catalogue::Current;
use crate::terrain::Terrain;

/// The shops and events of the game's own server.
const SHOPS: &str = include_str!("../../assets/shops.cfg");
const EVENTS: &str = include_str!("../../assets/events.cfg");
/// Traffic cars around the player.
const TRAFFIC: usize = crate::traffic::DEFAULT_CARS;
/// How long quitting waits for the server to save the player's car (s).
const LAST_SAVE: f32 = 4.0;

/// Where playing alone keeps what the player has (tests keep nothing).
fn database() -> Option<String> {
    if cfg!(test) {
        return Some("sqlite::memory:".into());
    }
    let dir = crate::storage::data_dir()?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("Couldn't make {}: {e}", dir.display());
        return None;
    }
    Some(format!("sqlite://{}", dir.join("alone.db").display()))
}

/// Starts the game's own server and joins it: the link is there at once (the server
/// answers once it's up).
pub fn start() -> LocalLink {
    let listener = LoopbackListener::default();
    let link = listener.connect();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let thread = std::thread::Builder::new()
        .name("local server".into())
        .spawn(move || {
            let mut app = server_app(Server::new(listener), TRAFFIC, false);
            let server = app.world_mut().remove_resource::<Server>().unwrap();
            let server = server
                .with_map(MapPlayers::Everyone)
                .with_chat(false)
                .with_max_players(1)
                .with_shops(crate::shops::parse(SHOPS).unwrap_or_else(|e| {
                    warn!("The game's own shops: {e}");
                    Vec::new()
                }))
                .with_events(events(app.world().resource::<Terrain>()));
            let accounts = database().map(|url| Accounts::alone(&url).map_err(|e| (url, e)));
            let server = match accounts {
                Some(Ok(accounts)) => server.with_accounts(accounts),
                Some(Err((url, e))) => {
                    warn!("Playing alone without keeping anything: {url}: {e}");
                    server
                }
                None => server,
            };
            app.insert_resource(server).add_systems(
                Update,
                move |server: Res<Server>,
                      mut peers: Query<&mut Peer>,
                      catalogue: Res<Current>,
                      mut saved: Local<bool>,
                      mut exit: MessageWriter<AppExit>| {
                    if stopping.load(Ordering::Relaxed) && !*saved {
                        *saved = true;
                        server.save_everyone(peers.iter_mut().map(|p| p.into_inner()), &catalogue);
                        exit.write(AppExit::Success);
                    }
                },
            );
            run_server(app);
        })
        .expect("a thread for the game's own server");
    LocalLink {
        link,
        stop,
        thread: Some(thread),
    }
}

/// The events of the game's own server, their routes found on `terrain`.
fn events(terrain: &Terrain) -> Vec<crate::events::Event> {
    let mut events = crate::events::parse(EVENTS).unwrap_or_else(|e| {
        warn!("The game's own events: {e}");
        Vec::new()
    });
    if let Err(e) = crate::events::resolve(&mut events, terrain) {
        warn!("The game's own events: {e}");
        events.clear();
    }
    events
}

/// The link to the game's own server, which stops the server when dropped.
pub struct LocalLink {
    link: Loopback,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalLink {
    /// Whether the server's thread has ended (stopped, or panicked).
    fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Link for LocalLink {
    fn send(&mut self, reliable: bool, message: Vec<u8>) {
        self.link.send(reliable, message);
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        self.link.receive()
    }

    fn is_open(&self) -> bool {
        self.link.is_open() && !self.ended()
    }

    fn close(&mut self) {
        self.link.close();
        self.stop.store(true, Ordering::Relaxed);
    }

    fn problem(&self) -> Option<String> {
        self.ended()
            .then(|| "The game's own server stopped (see the log)".to_string())
    }

    fn rtt(&self) -> Option<Duration> {
        Some(Duration::ZERO)
    }
}

impl Drop for LocalLink {
    fn drop(&mut self) {
        self.close();
        // The server saves the player's car before it stops: quitting waits for that a
        // little, not for ever.
        if let Some(thread) = self.thread.take() {
            let started = std::time::Instant::now();
            while !thread.is_finished() && started.elapsed().as_secs_f32() < LAST_SAVE {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{PROTOCOL, ToClient, ToServer};
    use super::super::wire::{decode, encode};
    use super::*;

    /// The game's own server starts, says welcome with its shops and events, and stops
    /// once its link goes.
    #[test]
    fn the_games_own_server_says_welcome() {
        let mut link = start();
        let terrain = Terrain::load();
        link.send(
            true,
            encode(&ToServer::Hello {
                protocol: PROTOCOL,
                map: terrain.fingerprint,
                name: "Anna".into(),
                password: String::new(),
            }),
        );
        let mut welcome = false;
        let mut shops = 0;
        for _ in 0..600 {
            while let Some(message) = link.receive() {
                match decode::<ToClient>(&message) {
                    Some(ToClient::Welcome { .. }) => welcome = true,
                    Some(ToClient::Shops(list)) => shops = list.len(),
                    _ => {}
                }
            }
            if shops > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(welcome, "no welcome");
        assert!(shops > 0, "no shops");
        let stop = link.stop.clone();
        let thread = link.thread.take().unwrap();
        drop(link);
        assert!(stop.load(Ordering::Relaxed));
        thread.join().unwrap();
    }
}
