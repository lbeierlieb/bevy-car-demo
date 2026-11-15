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
  (96161, Bavaria), with its drivable roads from OpenStreetMap. The terrain is
  flattened under the roads so they're smooth to drive on. The height and raycast
  queries use the exact rendered triangles.
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
| `tools/bake` | Separate crate that bakes `assets/*.terrain` from online data |
| `src/visuals.rs` | Procedural meshes posed from the simulation |
| `src/debug.rs`, `src/ui.rs`, `src/camera.rs` | Gizmos, telemetry panel, chase camera |

## Baking another location

`tools/bake` downloads elevation from the AWS Terrain Tiles and roads from the
OpenStreetMap API. It samples the elevation onto a 4 m grid, smooths each road's
height profile, and flattens the terrain beside the roads. The output is written
to a file that the game embeds at compile time:

```sh
nix develop
cd tools/bake
cargo build --release
./target/release/bake <lat> <lon> <size_m> ../../assets/<name>.terrain
```

Gerach was baked with `50.02955 10.80867 1600`. To use a different file, change
`BAKED` in `src/terrain.rs`. The OSM API only serves small areas, so keep the size
to a few kilometres.

## Deliberate simplifications

Massless arms with a point unsprung mass. No anti-roll bars, aerodynamics, gearbox
or tire relaxation length. No render interpolation between chassis steps. At very
low speed, slips are computed against a 3 m/s floor, so a parked car can creep
slowly down steep slopes. Elevation data is only about 30 m resolution, so
off-road terrain is smooth. Bridges become plain embankments.

## Data attribution

- Map data © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, available under the Open Database License (ODbL).
- Elevation via [Terrain Tiles](https://registry.opendata.aws/terrain-tiles/):
  Europe terrain data produced using Copernicus data and information funded by
  the European Union (EU-DEM layers); SRTM terrain data courtesy of the U.S.
  Geological Survey.
