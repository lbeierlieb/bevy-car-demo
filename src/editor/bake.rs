//! Re-baking from the editor (F5 or the panel's button): saves the map file, runs the
//! bake (`cargo run --release` in `tools/bake`, so it's built first if needed; the built
//! binary if there's no cargo) into `assets/gerach.terrain`, and reloads the map in place.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::prelude::*;

use super::conflicts::Conflicts;
use super::doc::Doc;
use super::tools::Tools;
use super::{CENTER, MAP_FILE, SIZE, repo_file};
use crate::map::MapView;
use crate::scenery::Reload;
use crate::terrain::{TERRAIN_FILE, Terrain};

/// About how long a bake takes (s), for the progress bar.
const TYPICAL: f32 = 25.0;
/// Output lines kept.
const KEEP_LINES: usize = 200;

pub struct BakePlugin;

impl Plugin for BakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bake>()
            .add_systems(Update, (start, poll));
    }
}

#[derive(Resource, Default)]
pub struct Bake {
    /// Set by the panel (or F5) to start one.
    pub requested: bool,
    job: Option<Job>,
    /// How the last one went: what it said last, and whether it worked.
    pub last: Option<(bool, String)>,
}

struct Job {
    child: Child,
    started: Instant,
    output: Arc<Mutex<Vec<String>>>,
}

impl Bake {
    /// While one runs: seconds since it started, and its last line of output.
    pub fn running(&self) -> Option<(f32, String)> {
        let job = self.job.as_ref()?;
        let last = job.output.lock().ok()?.last().cloned().unwrap_or_default();
        Some((job.started.elapsed().as_secs_f32(), last))
    }

    pub fn progress(elapsed: f32) -> f32 {
        (elapsed / TYPICAL).min(0.97)
    }
}

/// The bake's arguments: map file, centre, size, output.
fn arguments(out: &Path) -> Vec<String> {
    vec![
        repo_file(MAP_FILE).display().to_string(),
        CENTER.0.to_string(),
        CENTER.1.to_string(),
        SIZE.to_string(),
        out.display().to_string(),
    ]
}

fn start(
    keys: Res<ButtonInput<KeyCode>>,
    mut bake: ResMut<Bake>,
    mut doc: ResMut<Doc>,
    mut tools: ResMut<Tools>,
) {
    if keys.just_pressed(KeyCode::F5) {
        bake.requested = true;
    }
    if !std::mem::take(&mut bake.requested) || bake.job.is_some() {
        return;
    }
    if doc.dirty
        && let Err(e) = doc.save()
    {
        tools.status = format!("Couldn't save: {e}");
        return;
    }
    let out = Terrain::file().unwrap_or_else(|| repo_file(TERRAIN_FILE));
    // Built first if cargo is there (`cargo build`: the dev shell's `cargo run` adds a
    // profile of its own), where `tools/bake` builds it by itself, whatever this app was
    // built with; then run.
    let manifest = repo_file("tools/bake/Cargo.toml");
    let binary = repo_file("tools/bake/target/release/bake");
    let mut script = Command::new("sh");
    script
        .env_remove("CARGO_TARGET_DIR")
        .arg("-c")
        .arg(BUILD_AND_RUN)
        .arg(&binary)
        .arg(&manifest)
        .args(arguments(&out));
    let spawned = spawn(script).or_else(|_| {
        let mut built = Command::new(&binary);
        built.args(arguments(&out));
        spawn(built)
    });
    match spawned {
        Ok((child, output)) => {
            bake.job = Some(Job {
                child,
                started: Instant::now(),
                output,
            });
            bake.last = None;
            tools.status = "Baking…".into();
        }
        Err(e) => {
            bake.last = Some((false, format!("Couldn't start the bake: {e}")));
        }
    }
}

/// `sh -c` script: $0 is the bake's binary, $1 its manifest, the rest its arguments.
const BUILD_AND_RUN: &str = r#"if command -v cargo >/dev/null; then
    cargo build --release --manifest-path "$1" || exit 1
fi
shift
exec "$0" "$@""#;

type Output = Arc<Mutex<Vec<String>>>;

/// Starts a command with its output (both streams) collected line by line.
fn spawn(mut command: Command) -> std::io::Result<(Child, Output)> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;
    let output: Output = Arc::default();
    let streams: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|s| Box::new(s) as _),
        child.stderr.take().map(|s| Box::new(s) as _),
    ];
    for stream in streams.into_iter().flatten() {
        let output = output.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Ok(mut lines) = output.lock() {
                    lines.push(line);
                    if lines.len() > KEEP_LINES {
                        lines.remove(0);
                    }
                }
            }
        });
    }
    Ok((child, output))
}

#[allow(clippy::too_many_arguments)]
fn poll(
    mut commands: Commands,
    mut bake: ResMut<Bake>,
    mut tools: ResMut<Tools>,
    mut map: ResMut<MapView>,
    mut conflicts: ResMut<Conflicts>,
    mut reload: MessageWriter<Reload>,
    doc: Res<Doc>,
) {
    let Some(job) = &mut bake.job else {
        return;
    };
    let status = match job.child.try_wait() {
        Ok(None) => return,
        Ok(Some(status)) => status,
        Err(e) => {
            bake.job = None;
            bake.last = Some((false, format!("Lost the bake: {e}")));
            return;
        }
    };
    let seconds = job.started.elapsed().as_secs_f32();
    let lines = job.output.lock().map(|l| l.clone()).unwrap_or_default();
    bake.job = None;
    if !status.success() {
        let tail = lines[lines.len().saturating_sub(8)..].join("\n");
        bake.last = Some((false, format!("The bake failed ({status}):\n{tail}")));
        tools.status = "The bake failed".into();
        return;
    }
    let path = Terrain::file().unwrap_or_else(|| repo_file(TERRAIN_FILE));
    match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| Terrain::parse(&bytes))
    {
        Ok(terrain) => {
            commands.insert_resource(terrain);
            reload.write(Reload);
            map.rebuild();
            conflicts.reload(doc.frame);
            let summary = lines
                .iter()
                .rev()
                .find(|l| l.starts_with("wrote"))
                .cloned()
                .unwrap_or_default();
            bake.last = Some((true, format!("Baked in {seconds:.0} s. {summary}")));
            tools.status = "Baked and reloaded".into();
        }
        Err(e) => {
            bake.last = Some((false, format!("Couldn't load the new map: {e}")));
        }
    }
}
