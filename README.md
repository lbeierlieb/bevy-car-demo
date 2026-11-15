# Car Sim

A small Bevy demo of car simulation techniques:

- **Rigid-body chassis**: a hand-written rigid body (force/torque accumulation,
  semi-implicit Euler, gyroscopic term) integrated at 120 Hz. No physics engine.
- **Double A-arm suspension, simulated per corner at 1.2 kHz.** Each corner is a
  four-bar linkage (chassis, lower arm, upright, upper arm) with one degree of
  freedom, the lower arm angle θ. The upper ball joint comes from a circle
  intersection, so camber gain, track change and roll centre come out of the arm
  geometry. Steering rotates the upright about the kingpin axis. θ is integrated
  with a generalised force from virtual work (tire force at the contact patch,
  coilover spring and damper, bump stops). Whatever part of the hub force doesn't
  move the linkage goes straight into the chassis. Each chassis step, every corner
  runs 10 substeps against the chassis extrapolated at constant velocity, and the
  average force is applied to the chassis.
- **Raycast tires**: one ray per wheel, from the hub along the upright, against the
  terrain triangles. Tire load comes from carcass stiffness and damping. Forces come
  from a simplified Pacejka Magic Formula, with combined slip normalised by peak
  slip. Wheel spin is updated implicitly, which keeps it stable.
- **Engine and gearbox**: an engine with a torque curve, idle control, engine
  braking and a rev limiter that cuts the fuel in short bursts (so it bounces audibly
  off the redline) drives the rear wheels through a six-speed gearbox
  and an open differential. Once the clutch locks, the engine turns with the wheels
  and its inertia is added to theirs, so the wheel-spin update stays implicit. The
  clutch slips when pulling away and closes gradually after each 0.25 s shift, during
  which the throttle is cut. Gears are shifted by hand by default; the automatic
  shifts by rpm and throttle. The engine sound follows engine rpm, its volume the fuel the engine gets.
- **Real-world low-poly terrain**: a 9.6 × 8.4 km area of Upper Franconia,
  from Gerach (96161, Bavaria), Mauschendorf and Sendelbach in the north-west to
  Rattelsdorf and Ebing in the east and Baunach in the south, with Reckendorf,
  Laimbach, Höfen, Daschendorf, Freudeneck and more in between. Elevation comes from the Bavarian 1 m lidar terrain model. The map is split into 96 m chunks, each meshed at its own
  resolution: 2 m near buildings and paved roads, 4 m around farm tracks, 8 m
  elsewhere, with seams kept crack-free. Roads
  (with widths and surfaces from their OSM tags), streams, land use (fields,
  forest, meadows, gardens, water, parking areas) and buildings come from
  OpenStreetMap. Houses are boxes with gable roofs fitted to each footprint;
  they're visual only, with no collisions. Forests are filled with low-poly
  spruces and broadleaf trees (and scrub with bushes), scattered on a jittered
  7 m grid and kept clear of roads and streams. They're also visual only. The
  height and raycast queries use the exact rendered triangles.
- **Streamed meshes**: ground, roads, houses and trees are meshed per 96 m chunk
  when the chunk comes within 560 m of the camera, and freed again when it's left
  behind. GPU memory therefore doesn't grow with the map size.
- Three setups (baseline, soft, track), debug gizmos for the linkage and forces,
  and a telemetry panel with live-tunable springs, dampers, camber, arm geometry,
  tire parameters and brakes (torque and front/rear split). A HUD in the bottom
  right shows speed and gear inside a rev counter.
- A driver's view (C) from inside a low-poly cockpit, with a steering wheel that
  turns and a working rev counter and speedometer on the dashboard.

## Running

```sh
nix develop      # toolchain + native libraries
cargo run        # optimised build (release-dev profile) with fast rebuilds
cargo test       # physics unit tests and headless driving tests
```

Or without entering the shell: `nix run`. The web build is served with
`nix run .#serve-wasm` at <http://localhost:8080>. `nix flake check` builds both
targets and runs clippy, rustfmt, nixfmt and deadnix. GitHub Actions runs it on every
pull request, and every push to `main` deploys the web build to GitHub Pages.

## Controls

| Key | Action |
| --- | --- |
| W / S, arrows | Throttle; brake (a stopped car holds on hills) |
| Q / E | Shift down / up (R, N, 1–6) |
| G | Toggle automatic gearbox (S at a stop then selects reverse) |
| A / D | Steer |
| Space | Handbrake |
| Tab | Switch car |
| R | Reset car |
| F1 / F2 | Toggle gizmos / telemetry panel |
| M | Mute engine sound |
| C | Switch between chase and driver's view |
| Left-drag, scroll | Orbit and zoom the camera (driver's view: look around) |

### Steering wheel and pedals

Wheels, pedals and shifters work through the system's gamepad support; the keyboard
keeps working alongside. Open the panel (F2), expand "Wheel and pedals", and click
**Bind** next to each control, then move it: turn the wheel left, press a pedal fully
and release it, press a button. Pedals can rest at either end of their axis. The
bindings and settings are saved to `~/.config/car-sim/wheel.cfg` (on the web, in the
browser's local storage).

The wheel steers the front wheels directly at a fixed ratio: either end of the
rotation range is full lock, and the steering wheel in the cockpit turns with yours.
On Linux with the [hid-fanatecff](https://github.com/gotzl/hid-fanatecff) driver,
the game sets Fanatec wheel bases to the range on the slider (540° by default) and
restores their own range on exit; otherwise set the base to the same range by hand.
Writing the range needs the driver's udev rules (your user in the `games` group). The
paddles shift like Q/E (in automatic mode they select reverse or drive at a stop).

Gizmo colours: yellow for A-arm links, white for uprights, red for coilovers,
green for the raycast and contact point, blue for tire normal force, magenta for
tire grip force, cyan for roll centres.

## Code map

| File | Contents |
| --- | --- |
| `src/physics/rigid_body.rs` | Chassis rigid body |
| `src/physics/suspension.rs` | A-arm linkage, corner dynamics, wheel spin |
| `src/physics/tire.rs` | Magic Formula tire |
| `src/physics/drivetrain.rs` | Engine, clutch and gearbox |
| `src/sim.rs` | Multi-rate loop, controls to wheel torques, driving tests |
| `src/car.rs` | Setups, spawning, keyboard and wheel to controls |
| `src/input.rs` | Wheel, pedal and shifter bindings, config file |
| `src/wheel_base.rs` | Sets the wheel base's rotation range (Linux, hid-fanatecff) |
| `src/terrain.rs` | Baked map loading, terrain and road meshes, raycast, spawns |
| `src/buildings.rs` | Low-poly houses from the OSM building outlines (visual only) |
| `src/trees.rs` | Trees in forest areas and bushes on scrub (visual only) |
| `src/scenery.rs` | Loads and unloads chunk meshes around the camera |
| `tools/bake` | Separate crate that bakes `assets/*.terrain` from online data |
| `src/visuals.rs` | Procedural meshes posed from the simulation |
| `src/debug.rs`, `src/ui.rs`, `src/camera.rs` | Gizmos, telemetry panel, chase and driver cameras |
| `src/hud.rs` | Speed, gear and rev counter HUD |
| `src/cockpit.rs` | Interior for the driver's view: dashboard, steering wheel, gauges |

## Baking another location

`tools/bake` downloads the lidar terrain model (DGM1, 1 km tiles) from the
Bavarian surveying administration's open data server, and roads, land use and
buildings from OpenStreetMap with a single Overpass API query. Downloads are cached
in `tools/bake/cache` (delete it to fetch fresh data), so re-baking is quick and
doesn't load the servers again. The map frame is aligned with UTM zone 32,
like the lidar tiles. Elevation is box-filtered onto a 2 m grid and land use is
rasterised per cell. Bridges are rebuilt as straight decks, because lidar shows
the ground under them.

Each 96 m chunk is then given a resolution. It gets 2 m within 20 m of a building
or 10 m of a paved road or parking area, and 4 m within 6 m of a track. Everything
else gets 8 m, with each coarse vertex averaging the terrain around it. Along seams
between resolutions, the finer side's edge is bent onto the coarser side's straight
edge, so there are no cracks. Chunks are stored at their own resolution. The game
rebuilds the full 2 m height grid from them, with coarse areas lying exactly on
their coarse triangles, so physics still matches what's drawn.

The map is a rectangle centred on the given point, with both sides rounded to whole
chunks; give one size for a square. The output is written to a file that the game
embeds at compile time. DGM1 only
covers Bavaria, so locations elsewhere would need another elevation source.

```sh
nix develop
cd tools/bake
cargo build --release
./target/release/bake <lat> <lon> <width_m>[x<height_m>] ../../assets/<name>.terrain \
    [<spawn_lat> <spawn_lon> [<spawn_heading>]]
```

The map was baked with `50.0118569 10.8455963 9600x8448 ../../assets/gerach.terrain
50.0305556 10.8046389 180`, which starts the cars on the road nearest that point in
Gerach, facing as close to south (180°) as the road allows, one behind the other. To use a different file,
change `BAKED` in `src/terrain.rs`. The public Overpass server sometimes answers
that it's busy; the tool then retries every minute, up to five times. Only chunks near the camera are on the GPU, so map size is limited by
the size of the baked file, which is embedded in the program (about 0.65 MB per km²
with the current mix of resolutions).

## Deliberate simplifications

Massless arms with a point unsprung mass. No anti-roll bars, aerodynamics or tire
relaxation length (so in top gear the car runs to the rev limiter). The clutch is
automatic, and the engine stays coupled to both rear wheels' average speed. No render interpolation between chassis steps. At very
low speed, slips are computed against a 3 m/s floor, so a parked car can creep
slowly down steep slopes. Bridges become plain embankments. Land use is coloured
per cell (2–8 m), so area edges are stepped, most visibly in coarse chunks.

## Data attribution

- Map data © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, available under the Open Database License (ODbL).
- Elevation: Digitales Geländemodell 1 m (DGM1) © Bayerische
  Vermessungsverwaltung, [geodaten.bayern.de](https://geodaten.bayern.de/opengeodata/),
  licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
