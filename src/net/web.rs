//! WebTransport in the browser, joining a server: the browser's own `WebTransport`,
//! bound here by hand (web-sys has it only behind an unstable flag). Like natively, one
//! bidirectional stream carries the reliable messages (each framed by its length) and
//! datagrams the rest. The JavaScript objects live in the connection's task (the page has
//! one thread); the game's `WebLink` only shares queues with it.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use js_sys::{Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    ReadableStream, ReadableStreamDefaultReader, WritableStream, WritableStreamDefaultWriter,
};

use super::link::Link;

/// The longest message taken off the stream (bytes), and the largest datagram sent.
const MAX_MESSAGE: usize = 1 << 20;
const MAX_DATAGRAM: usize = 1200;

#[wasm_bindgen]
extern "C" {
    type WebTransport;
    #[wasm_bindgen(constructor, catch)]
    fn new(url: &str) -> Result<WebTransport, JsValue>;
    #[wasm_bindgen(method, getter)]
    fn ready(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn closed(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn datagrams(this: &WebTransport) -> Datagrams;
    #[wasm_bindgen(method, js_name = createBidirectionalStream)]
    fn create_bidirectional_stream(this: &WebTransport) -> Promise;
    #[wasm_bindgen(method)]
    fn close(this: &WebTransport);

    type Datagrams;
    #[wasm_bindgen(method, getter)]
    fn readable(this: &Datagrams) -> ReadableStream;
    #[wasm_bindgen(method, getter)]
    fn writable(this: &Datagrams) -> WritableStream;
    #[wasm_bindgen(method, getter, js_name = maxDatagramSize)]
    fn max_datagram_size(this: &Datagrams) -> u32;

    type BidirectionalStream;
    #[wasm_bindgen(method, getter)]
    fn readable(this: &BidirectionalStream) -> ReadableStream;
    #[wasm_bindgen(method, getter)]
    fn writable(this: &BidirectionalStream) -> WritableStream;
}

/// What the game and the connection's task share.
#[derive(Default)]
struct Shared {
    incoming: VecDeque<Vec<u8>>,
    /// Sent before the connection stood.
    waiting: Vec<(bool, Vec<u8>)>,
    problem: Option<String>,
}

/// A standing connection's JavaScript side, by link number.
struct Writers {
    transport: WebTransport,
    stream: WritableStreamDefaultWriter,
    datagrams: WritableStreamDefaultWriter,
    largest: usize,
}

thread_local! {
    static WRITERS: RefCell<HashMap<u32, Writers>> = RefCell::new(HashMap::new());
}

/// The game's end of a connection.
pub struct WebLink {
    id: u32,
    shared: Arc<Mutex<Shared>>,
    open: Arc<AtomicBool>,
}

impl Link for WebLink {
    fn send(&mut self, reliable: bool, message: Vec<u8>) {
        if !self.is_open() {
            return;
        }
        let sent = WRITERS.with_borrow(|writers| {
            let Some(w) = writers.get(&self.id) else {
                return false;
            };
            write(w, reliable, &message);
            true
        });
        if !sent {
            self.shared
                .lock()
                .unwrap()
                .waiting
                .push((reliable, message));
        }
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        self.shared.lock().unwrap().incoming.pop_front()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    fn close(&mut self) {
        self.open.store(false, Ordering::Relaxed);
        if let Some(w) = WRITERS.with_borrow_mut(|writers| writers.remove(&self.id)) {
            w.transport.close();
        }
    }

    fn problem(&self) -> Option<String> {
        self.shared.lock().unwrap().problem.clone()
    }
}

impl Drop for WebLink {
    fn drop(&mut self) {
        self.close();
    }
}

/// Writes a message (the promise is left to itself; writes queue up in order).
fn write(w: &Writers, reliable: bool, message: &[u8]) {
    let ignore = Closure::<dyn FnMut(JsValue)>::new(|_| {});
    let promise = if reliable || message.len() > w.largest {
        let mut framed = (message.len() as u32).to_le_bytes().to_vec();
        framed.extend_from_slice(message);
        w.stream.write_with_chunk(&Uint8Array::from(&framed[..]))
    } else {
        w.datagrams.write_with_chunk(&Uint8Array::from(message))
    };
    let _ = promise.catch(&ignore);
    ignore.forget();
}

/// Joins the server at `url` (`https://host:port`). The link is there at once; messages
/// sent before the connection stands wait for it.
pub fn connect(url: &str) -> WebLink {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let link = WebLink {
        id: NEXT.fetch_add(1, Ordering::Relaxed),
        shared: Arc::default(),
        open: Arc::new(AtomicBool::new(true)),
    };
    let (id, shared, open) = (link.id, link.shared.clone(), link.open.clone());
    let url = url.to_string();
    spawn_local(async move {
        let fail = |problem: String| {
            shared.lock().unwrap().problem.get_or_insert(problem);
            open.store(false, Ordering::Relaxed);
            WRITERS.with_borrow_mut(|writers| writers.remove(&id));
        };
        let started = async {
            let transport = WebTransport::new(&url)?;
            JsFuture::from(transport.ready()).await?;
            let stream: BidirectionalStream =
                JsFuture::from(transport.create_bidirectional_stream())
                    .await?
                    .unchecked_into();
            Ok::<_, JsValue>((transport, stream))
        };
        let (transport, stream) = match started.await {
            Ok(started) => started,
            Err(e) => return fail(format!("Couldn't join {url}: {}", describe(&e))),
        };
        if !open.load(Ordering::Relaxed) {
            // Let go of before it stood.
            transport.close();
            return;
        }
        let datagrams = transport.datagrams();
        let writers = Writers {
            stream: stream.writable().get_writer().unwrap(),
            datagrams: datagrams.writable().get_writer().unwrap(),
            largest: (datagrams.max_datagram_size() as usize).min(MAX_DATAGRAM),
            transport,
        };
        for (reliable, message) in std::mem::take(&mut shared.lock().unwrap().waiting) {
            write(&writers, reliable, &message);
        }
        let closed = writers.transport.closed();
        WRITERS.with_borrow_mut(|w| w.insert(id, writers));

        // Reading: the stream's bytes into messages, the datagrams as they are.
        let into = shared.clone();
        let reading = async move {
            let reader: ReadableStreamDefaultReader =
                stream.readable().get_reader().unchecked_into();
            let mut bytes: Vec<u8> = Vec::new();
            while let Some(chunk) = read(&reader).await? {
                bytes.extend_from_slice(&chunk);
                while bytes.len() >= 4 {
                    let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
                    if length > MAX_MESSAGE {
                        return Err(JsValue::from_str("a message too long"));
                    }
                    if bytes.len() < 4 + length {
                        break;
                    }
                    let message = bytes[4..4 + length].to_vec();
                    bytes.drain(..4 + length);
                    into.lock().unwrap().incoming.push_back(message);
                }
            }
            Ok(())
        };
        let into = shared.clone();
        spawn_local(async move {
            let reader: ReadableStreamDefaultReader =
                datagrams.readable().get_reader().unchecked_into();
            while let Ok(Some(datagram)) = read(&reader).await {
                into.lock().unwrap().incoming.push_back(datagram);
            }
        });
        let ended = reading.await;
        let _ = JsFuture::from(closed).await;
        match ended {
            Ok(()) => fail("The server closed the connection".into()),
            Err(e) => fail(describe(&e)),
        }
    });
    link
}

/// The next chunk from a stream, or nothing once it's done.
async fn read(reader: &ReadableStreamDefaultReader) -> Result<Option<Vec<u8>>, JsValue> {
    let result = JsFuture::from(reader.read()).await?;
    if Reflect::get(&result, &"done".into())?.as_bool() == Some(true) {
        return Ok(None);
    }
    let value: Uint8Array = Reflect::get(&result, &"value".into())?.unchecked_into();
    Ok(Some(value.to_vec()))
}

fn describe(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// The page's own host, which a web build served with the server joins by default.
pub fn page_host() -> Option<String> {
    web_sys::window()?.location().hostname().ok()
}
