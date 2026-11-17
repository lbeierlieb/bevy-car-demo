//! Baking an engine's maps (a few seconds for parts on the cylinder side): on a thread, or
//! on the web a slice every frame. Baked maps are cached (`~/.cache/car-sim/engines`,
//! local storage on the web), so going back to a build is quick.

#[cfg(target_arch = "wasm32")]
use bevy::platform::time::Instant;

use crate::physics::engine::hardware::EngineHardware;
use crate::physics::engine::maps::{self, Bake, EngineMaps};
use crate::storage::Folder;

const MAPS: Folder = Folder {
    cache: true,
    name: "engines",
    extension: "maps",
    web_prefix: "car-sim-maps:",
};

/// Grid points baked per batch on the thread, and time spent baking per frame on the web.
#[cfg(not(target_arch = "wasm32"))]
const BATCH: usize = 64;
#[cfg(target_arch = "wasm32")]
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(12);

/// The maps for `hardware` if they needn't be baked: a factory engine's, or cached ones.
pub fn at_hand(hardware: &EngineHardware) -> Option<EngineMaps> {
    EngineMaps::stock(hardware).or_else(|| {
        let text = MAPS.load(&cache_name(hardware))?;
        EngineMaps::from_text(&text, maps::fingerprint(hardware))
    })
}

/// Caches the maps baked for `hardware`.
pub fn keep(hardware: &EngineHardware, maps: &EngineMaps) {
    let _ = MAPS.save(&cache_name(hardware), &maps.to_text(hardware));
}

fn cache_name(hardware: &EngineHardware) -> String {
    format!("{:016x}", maps::fingerprint(hardware))
}

/// Baking on a thread, which reports its progress (an f32's bits) and can be told to stop.
#[cfg(not(target_arch = "wasm32"))]
pub struct Work {
    progress: std::sync::Arc<std::sync::atomic::AtomicU32>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Option<EngineMaps>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Work {
    pub fn start(hardware: &EngineHardware) -> Self {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut bake = Bake::new(hardware);
        let (shown, stop) = (progress.clone(), cancel.clone());
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(maps) = bake.advance(BATCH) {
                    return Some(maps);
                }
                shown.store(bake.progress().to_bits(), Ordering::Relaxed);
            }
            None
        });
        Self {
            progress,
            cancel,
            thread: Some(thread),
        }
    }

    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// The maps once they're done, or nothing if baking failed.
    pub fn poll(&mut self) -> Option<Option<EngineMaps>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().ok().flatten())
    }

    pub fn cancel(self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Baking a slice every frame (there are no threads on the web).
#[cfg(target_arch = "wasm32")]
pub struct Work {
    bake: Bake,
}

#[cfg(target_arch = "wasm32")]
impl Work {
    pub fn start(hardware: &EngineHardware) -> Self {
        Self {
            bake: Bake::new(hardware),
        }
    }

    pub fn progress(&self) -> f32 {
        self.bake.progress()
    }

    pub fn poll(&mut self) -> Option<Option<EngineMaps>> {
        let start = Instant::now();
        while start.elapsed() < FRAME_BUDGET {
            if let Some(maps) = self.bake.advance(1) {
                return Some(Some(maps));
            }
        }
        None
    }

    pub fn cancel(self) {}
}
