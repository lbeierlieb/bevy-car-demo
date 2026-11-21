//! WebTransport natively (the server, and the native game joining one), through
//! wtransport on a small tokio runtime of its own. Each connection has one bidirectional
//! stream for the reliable messages (each framed by its length) and datagrams for the
//! rest (or the stream, for one too big for a datagram). The game side of a connection is
//! a `QuicLink`: queues to and from the connection's task.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use bevy::log::{info, warn};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use wtransport::endpoint::IncomingSession;
use wtransport::{ClientConfig, Endpoint, Identity, RecvStream, SendStream, ServerConfig};

use super::link::{Link, Listener};

/// The longest message taken off the stream (bytes).
const MAX_MESSAGE: usize = 1 << 20;
/// A datagram is never larger than this (bytes), whatever the path allows.
const MAX_DATAGRAM: usize = 1200;
/// Keep-alives, and how long a silent connection lasts.
const KEEP_ALIVE: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_secs(15);
/// How long joining may take, and how often the server looks for a renewed certificate.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CERTIFICATE_CHECK: Duration = Duration::from_secs(60);
/// How often the round-trip time is read off the connection.
const RTT_CHECK: Duration = Duration::from_millis(500);

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("network")
            .enable_all()
            .build()
            .expect("couldn't start the network runtime")
    })
}

/// The game's end of a connection.
pub struct QuicLink {
    outgoing: Option<UnboundedSender<(bool, Vec<u8>)>>,
    incoming: UnboundedReceiver<Vec<u8>>,
    open: Arc<AtomicBool>,
    problem: Arc<Mutex<Option<String>>>,
    /// The round-trip time (µs; 0 until the connection stands).
    rtt: Arc<AtomicU32>,
}

/// The connection task's end.
struct Ends {
    outgoing: UnboundedReceiver<(bool, Vec<u8>)>,
    incoming: UnboundedSender<Vec<u8>>,
    open: Arc<AtomicBool>,
    problem: Arc<Mutex<Option<String>>>,
    rtt: Arc<AtomicU32>,
}

fn link() -> (QuicLink, Ends) {
    let (to_net, outgoing) = unbounded_channel();
    let (incoming, from_net) = unbounded_channel();
    let open = Arc::new(AtomicBool::new(true));
    let problem = Arc::new(Mutex::new(None));
    let rtt = Arc::new(AtomicU32::new(0));
    (
        QuicLink {
            outgoing: Some(to_net),
            incoming: from_net,
            open: open.clone(),
            problem: problem.clone(),
            rtt: rtt.clone(),
        },
        Ends {
            outgoing,
            incoming,
            open,
            problem,
            rtt,
        },
    )
}

impl Ends {
    fn fail(&self, problem: String) {
        self.problem.lock().unwrap().get_or_insert(problem);
        self.open.store(false, Ordering::Relaxed);
    }
}

impl Link for QuicLink {
    fn send(&mut self, reliable: bool, message: Vec<u8>) {
        if let Some(outgoing) = &self.outgoing {
            let _ = outgoing.send((reliable, message));
        }
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        self.incoming.try_recv().ok()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        // The task closes the connection once nothing more can be sent.
        self.outgoing = None;
        self.open.store(false, Ordering::Relaxed);
    }

    fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }

    fn rtt(&self) -> Option<Duration> {
        let micros = self.rtt.load(Ordering::Relaxed);
        (micros > 0).then(|| Duration::from_micros(micros.into()))
    }
}

impl Drop for QuicLink {
    fn drop(&mut self) {
        self.close();
    }
}

/// Carries messages over an established connection until either side ends it.
async fn run(
    connection: wtransport::Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    mut ends: Ends,
) {
    let incoming = ends.incoming.clone();
    let reliable = async {
        loop {
            let mut length = [0; 4];
            recv.read_exact(&mut length)
                .await
                .map_err(|e| e.to_string())?;
            let length = u32::from_le_bytes(length) as usize;
            if length > MAX_MESSAGE {
                return Err(format!("a message of {length} bytes"));
            }
            let mut message = vec![0; length];
            recv.read_exact(&mut message)
                .await
                .map_err(|e| e.to_string())?;
            if incoming.send(message).is_err() {
                return Ok(());
            }
        }
    };
    let datagrams = async {
        loop {
            let datagram = connection
                .receive_datagram()
                .await
                .map_err(|e| e.to_string())?;
            if ends.incoming.send(datagram.payload().to_vec()).is_err() {
                return Ok(());
            }
        }
    };
    let largest = connection
        .max_datagram_size()
        .unwrap_or(0)
        .min(MAX_DATAGRAM);
    let outgoing = &mut ends.outgoing;
    let writing = async {
        while let Some((reliable, message)) = outgoing.recv().await {
            if reliable || message.len() > largest {
                let length = (message.len() as u32).to_le_bytes();
                send.write_all(&length).await.map_err(|e| e.to_string())?;
                send.write_all(&message).await.map_err(|e| e.to_string())?;
            } else {
                // A lost one is soon replaced.
                let _ = connection.send_datagram(message);
            }
        }
        // The game let go of it.
        Ok(())
    };
    let rtt = ends.rtt.clone();
    let measuring = async {
        loop {
            let micros = connection.rtt().as_micros().clamp(1, u32::MAX.into()) as u32;
            rtt.store(micros, Ordering::Relaxed);
            tokio::time::sleep(RTT_CHECK).await;
        }
    };
    let ended: Result<(), String> = tokio::select! {
        r = reliable => r,
        r = datagrams => r,
        r = writing => r,
        r = measuring => r,
    };
    connection.close(0u32.into(), b"bye");
    match ended {
        Ok(()) => ends.fail("closed".into()),
        Err(e) => ends.fail(e),
    }
}

/// Joins the server at `url` (`https://host:port`). The link is there at once; messages
/// sent before the connection stands wait for it. `verify: false` accepts any certificate
/// (a development server's throwaway one).
pub fn connect(url: &str, verify: bool) -> QuicLink {
    let (link, ends) = link();
    let url = url.to_string();
    runtime().spawn(async move {
        let joined = tokio::time::timeout(CONNECT_TIMEOUT, async {
            let config = ClientConfig::builder().with_bind_default();
            let config = if verify {
                config.with_native_certs()
            } else {
                config.with_no_cert_validation()
            };
            let config = config
                .keep_alive_interval(Some(KEEP_ALIVE))
                .max_idle_timeout(Some(IDLE))
                .map_err(|e| e.to_string())?
                .build();
            let endpoint = Endpoint::client(config).map_err(|e| e.to_string())?;
            let connection = endpoint.connect(&url).await.map_err(|e| e.to_string())?;
            let (send, recv) = connection
                .open_bi()
                .await
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((connection, send, recv))
        })
        .await;
        match joined {
            Ok(Ok((connection, send, recv))) => run(connection, send, recv, ends).await,
            Ok(Err(e)) => ends.fail(format!("Couldn't join {url}: {e}")),
            Err(_) => ends.fail(format!("Couldn't join {url}: no answer")),
        }
    });
    link
}

/// The server's certificate.
#[derive(Clone, Debug)]
pub enum Certificate {
    /// PEM files (the chain and the key), read again when they change (renewals).
    Files { chain: PathBuf, key: PathBuf },
    /// A throwaway self-signed one for development: browsers won't join, the native game
    /// only without verifying.
    Throwaway,
}

impl Certificate {
    async fn identity(&self) -> Result<Identity, String> {
        match self {
            Certificate::Files { chain, key } => Identity::load_pemfiles(chain, key)
                .await
                .map_err(|e| format!("couldn't read the certificate: {e}")),
            Certificate::Throwaway => {
                Identity::self_signed(["localhost", "127.0.0.1", "::1"]).map_err(|e| e.to_string())
            }
        }
    }

    /// When the files last changed.
    fn changed(&self) -> Option<SystemTime> {
        match self {
            Certificate::Files { chain, key } => [chain, key]
                .iter()
                .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
                .max(),
            Certificate::Throwaway => None,
        }
    }
}

fn server_config(port: u16, identity: Identity) -> ServerConfig {
    ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(Some(IDLE))
        .expect("a valid idle timeout")
        .build()
}

/// The server's new connections, on a UDP port.
pub struct QuicListener {
    new: UnboundedReceiver<QuicLink>,
}

impl QuicListener {
    /// Listens on `port` (all addresses, IPv4 and IPv6) with `certificate`.
    pub fn bind(port: u16, certificate: Certificate) -> Result<Self, String> {
        let identity = runtime().block_on(certificate.identity())?;
        let endpoint = runtime()
            .block_on(async { Endpoint::server(server_config(port, identity)) })
            .map_err(|e| format!("couldn't listen on port {port}: {e}"))?;
        let endpoint = Arc::new(endpoint);
        let (found, new) = unbounded_channel();
        // Renewed certificates are taken up without a restart.
        let renewing = endpoint.clone();
        runtime().spawn(async move {
            let mut seen = certificate.changed();
            loop {
                tokio::time::sleep(CERTIFICATE_CHECK).await;
                let now = certificate.changed();
                if now == seen {
                    continue;
                }
                match certificate.identity().await {
                    Ok(identity) => {
                        match renewing.reload_config(server_config(port, identity), false) {
                            Ok(()) => {
                                info!("Took up the renewed certificate");
                                seen = now;
                            }
                            Err(e) => warn!("Couldn't take up the renewed certificate: {e}"),
                        }
                    }
                    Err(e) => warn!("{e}"),
                }
            }
        });
        runtime().spawn(async move {
            loop {
                let incoming = endpoint.accept().await;
                let found = found.clone();
                tokio::spawn(async move {
                    if let Err(e) = accept(incoming, found).await {
                        info!("A connection failed: {e}");
                    }
                });
            }
        });
        Ok(Self { new })
    }
}

async fn accept(incoming: IncomingSession, found: UnboundedSender<QuicLink>) -> Result<(), String> {
    let joined = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let request = incoming.await.map_err(|e| e.to_string())?;
        let connection = request.accept().await.map_err(|e| e.to_string())?;
        let (send, recv) = connection.accept_bi().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((connection, send, recv))
    })
    .await
    .map_err(|_| "it didn't open its stream".to_string())??;
    let (connection, send, recv) = joined;
    let (link, ends) = link();
    found.send(link).map_err(|_| "the server stopped")?;
    run(connection, send, recv, ends).await;
    Ok(())
}

impl Listener for QuicListener {
    fn accept(&mut self) -> Option<Box<dyn Link>> {
        let link = self.new.try_recv().ok()?;
        Some(Box::new(link))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server with a throwaway certificate on localhost, and a client joining it: what
    /// each sends comes through, reliably and as datagrams.
    #[test]
    fn messages_go_both_ways_over_webtransport() {
        let mut listener = QuicListener::bind(47_433, Certificate::Throwaway).unwrap();
        let mut client = connect("https://127.0.0.1:47433", false);
        client.send(true, b"hello".to_vec());
        let wait = |what: &mut dyn FnMut() -> bool| {
            let start = std::time::Instant::now();
            while !what() {
                assert!(start.elapsed() < Duration::from_secs(10), "timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let mut server: Option<Box<dyn Link>> = None;
        wait(&mut || {
            server = server.take().or_else(|| listener.accept());
            server.is_some()
        });
        let mut server = server.unwrap();
        let mut got = None;
        wait(&mut || {
            got = server.receive();
            got.is_some()
        });
        assert_eq!(got.as_deref(), Some(&b"hello"[..]));
        // Datagrams may be lost, even here: send a few.
        let mut got = None;
        wait(&mut || {
            server.send(false, b"state".to_vec());
            got = client.receive();
            got.is_some()
        });
        assert_eq!(got.as_deref(), Some(&b"state"[..]));
        // A big one goes over the stream.
        let big = vec![7; 5000];
        server.send(false, big.clone());
        let mut got = None;
        wait(&mut || {
            got = client.receive().filter(|m| m.len() > 5);
            got.is_some()
        });
        assert_eq!(got, Some(big));
        client.close();
        wait(&mut || !server.is_open());
    }

    /// Joins the real server at `CAR_SIM_SERVER` (e.g. `cars.example.org`) and prints
    /// what it answers to a hello with this build's protocol and map: for checking a
    /// deployed server from outside.
    /// `CAR_SIM_SERVER=cars.example.org cargo test join_a_real_server -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn join_a_real_server() {
        use crate::net::protocol::{PROTOCOL, ToClient, ToServer};
        use crate::net::wire::{decode, encode};
        use crate::terrain::Terrain;
        let address = std::env::var("CAR_SIM_SERVER").expect("CAR_SIM_SERVER=host");
        let url = crate::net::url(&address);
        println!("Joining {url}");
        let mut link = connect(&url, std::env::var_os("CAR_SIM_INSECURE").is_none());
        let hello = ToServer::Hello {
            protocol: PROTOCOL,
            map: Terrain::baked().fingerprint,
            name: "connection test".into(),
        };
        link.send(true, encode(&hello));
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            if let Some(bytes) = link.receive() {
                println!("The server says: {:?}", decode::<ToClient>(&bytes));
                link.close();
                return;
            }
            if !link.is_open() {
                panic!("{}", link.problem().unwrap_or_default());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("no answer in 15 s");
    }
}
