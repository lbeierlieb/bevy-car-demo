//! A connection between a player and the server, whatever carries it: a `Link` sends and
//! receives whole messages, reliably (in order, never lost) or not (may be lost, but
//! never late behind a lost one). The server takes new links from a `Listener`.
//! `loopback` connects them in memory (tests); WebTransport is the real one.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One end of a connection.
pub trait Link: Send + Sync + 'static {
    fn send(&mut self, reliable: bool, message: Vec<u8>);
    /// The next message that came in, if any.
    fn receive(&mut self) -> Option<Vec<u8>>;
    /// Whether the other end is still there.
    fn is_open(&self) -> bool;
    fn close(&mut self);
    /// What went wrong, once it's closed.
    fn problem(&self) -> Option<String> {
        None
    }
    /// The connection's round-trip time, as far as it's known.
    fn rtt(&self) -> Option<Duration> {
        None
    }
    /// Whether the other end is a web page (the server's end only).
    fn browser(&self) -> bool {
        false
    }
}

/// Where the server's new connections come from.
pub trait Listener: Send + Sync + 'static {
    fn accept(&mut self) -> Option<Box<dyn Link>>;
}

/// Messages in flight one way, in memory.
type Queue = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// One end of an in-memory connection.
pub struct Loopback {
    outgoing: Queue,
    incoming: Queue,
    open: Arc<AtomicBool>,
    browser: bool,
}

impl Loopback {
    /// The two ends of a new connection.
    pub fn pair() -> (Loopback, Loopback) {
        let (a, b) = (Queue::default(), Queue::default());
        let open = Arc::new(AtomicBool::new(true));
        (
            Loopback {
                outgoing: a.clone(),
                incoming: b.clone(),
                open: open.clone(),
                browser: false,
            },
            Loopback {
                outgoing: b,
                incoming: a,
                open,
                browser: false,
            },
        )
    }
}

impl Link for Loopback {
    fn send(&mut self, _reliable: bool, message: Vec<u8>) {
        if self.is_open() {
            self.outgoing.lock().unwrap().push_back(message);
        }
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        self.incoming.lock().unwrap().pop_front()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        self.open.store(false, Ordering::Relaxed);
    }

    /// In memory: none.
    fn rtt(&self) -> Option<Duration> {
        Some(Duration::ZERO)
    }

    fn browser(&self) -> bool {
        self.browser
    }
}

/// A server's end of in-memory connections: `connect` makes one and hands back the
/// player's end.
#[derive(Clone, Default)]
pub struct LoopbackListener {
    pending: Arc<Mutex<Vec<Loopback>>>,
}

impl LoopbackListener {
    pub fn connect(&self) -> Loopback {
        let (player, server) = Loopback::pair();
        self.pending.lock().unwrap().push(server);
        player
    }

    /// A connection the server takes for one from a web page.
    pub fn connect_from_browser(&self) -> Loopback {
        let player = self.connect();
        self.pending.lock().unwrap().last_mut().unwrap().browser = true;
        player
    }
}

impl Listener for LoopbackListener {
    fn accept(&mut self) -> Option<Box<dyn Link>> {
        let link = self.pending.lock().unwrap().pop()?;
        Some(Box::new(link))
    }
}

impl Link for Box<dyn Link> {
    fn send(&mut self, reliable: bool, message: Vec<u8>) {
        (**self).send(reliable, message);
    }
    fn receive(&mut self) -> Option<Vec<u8>> {
        (**self).receive()
    }
    fn is_open(&self) -> bool {
        (**self).is_open()
    }
    fn close(&mut self) {
        (**self).close();
    }
    fn problem(&self) -> Option<String> {
        (**self).problem()
    }
    fn rtt(&self) -> Option<Duration> {
        (**self).rtt()
    }
    fn browser(&self) -> bool {
        (**self).browser()
    }
}
