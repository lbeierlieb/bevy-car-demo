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
- **Real-world low-poly terrain**: a 1.6 km square around the village of Gerach
  (96161, Bavaria), meshed at 2 m from the Bavarian 1 m lidar terrain model. Roads
  (with widths and surfaces from their OSM tags), streams, land use (fields,
  forest, meadows, gardens, water, parking areas) and buildings come from
  OpenStreetMap. Houses are boxes with gable roofs fitted to each footprint;
  they're visual only, with no collisions. The height and raycast queries use the
  exact rendered triangles.
- Three setups (baseline, soft, track), debug gizmos for the linkage and forces,
  and a telemetry panel with live-tunable springs, dampers, camber, arm geometry
  and tire parameters.

## Running

```sh
nix develop      # toolchain + native libraries
cargo run        # optimised build (release-dev profile) with fast rebuilds
cargo test       # physics unit tests and headless driving tests
```

Or without entering the shell: `nix run`. The web build is served with
`nix run .#serve-wasm` at <http://localhost:8080>. `nix flake check` builds both
targets and runs clippy, rustfmt, nixfmt and deadnix.

## Controls

| Key | Action |
| --- | --- |
| W / S, arrows | Throttle; brake, then reverse |
| A / D | Steer |
| Space | Handbrake |
| Tab | Switch car |
| R | Reset car |
| F1 / F2 | Toggle gizmos / telemetry panel |
| M | Mute engine sound |
| Left-drag, scroll | Orbit and zoom the camera |

Gizmo colours: yellow for A-arm links, white for uprights, red for coilovers,
green for the raycast and contact point, blue for tire normal force, magenta for
tire grip force, cyan for roll centres.

## Code map

| File | Contents |
| --- | --- |
| `src/physics/rigid_body.rs` | Chassis rigid body |
| `src/physics/suspension.rs` | A-arm linkage, corner dynamics, wheel spin |
| `src/physics/tire.rs` | Magic Formula tire |
| `src/sim.rs` | Multi-rate loop, drivetrain, driving tests |
| `src/car.rs` | Setups, spawning, keyboard input |
| `src/terrain.rs` | Baked map loading, terrain and road meshes, raycast, spawns |
| `src/buildings.rs` | Low-poly houses from the OSM building outlines (visual only) |
| `tools/bake` | Separate crate that bakes `assets/*.terrain` from online data |
| `src/visuals.rs` | Procedural meshes posed from the simulation |
| `src/debug.rs`, `src/ui.rs`, `src/camera.rs` | Gizmos, telemetry panel, chase camera |

## Baking another location

`tools/bake` downloads the lidar terrain model (DGM1, 1 km tiles) from the
Bavarian surveying administration's open data server, and roads, land use and
buildings from the OpenStreetMap API. The map frame is aligned with UTM zone 32,
like the lidar tiles. Elevation is box-filtered onto a 2 m grid and land use is
rasterised per cell. Bridges are rebuilt as straight decks, because lidar shows
the ground under them. The output is written to a file that the game embeds at
compile time. DGM1 only covers Bavaria, so locations elsewhere would need another
elevation source.

```sh
nix develop
cd tools/bake
cargo build --release
./target/release/bake <lat> <lon> <size_m> ../../assets/<name>.terrain
```

Gerach was baked with `50.02955 10.80867 1600`. To use a different file, change
`BAKED` in `src/terrain.rs`. The OSM API refuses requests with more than 50 000 nodes;
the tool then splits the area into quarters (recursively) and merges the results.
Map size is limited more by GPU memory: about 65 MB per km² of terrain mesh.

## Deliberate simplifications

Massless arms with a point unsprung mass. No anti-roll bars, aerodynamics, gearbox
or tire relaxation length. No render interpolation between chassis steps. At very
low speed, slips are computed against a 3 m/s floor, so a parked car can creep
slowly down steep slopes. Bridges become plain embankments. Land use is coloured
per 2 m cell, so area edges are slightly stepped.

## Data attribution

- Map data © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, available under the Open Database License (ODbL).
- Elevation: Digitales Geländemodell 1 m (DGM1) © Bayerische
  Vermessungsverwaltung, [geodaten.bayern.de](https://geodaten.bayern.de/opengeodata/),
  licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
