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
  off the redline) drives the rear wheels, the front wheels or all four through the
  car's gearbox and open or limited-slip differentials. Once the clutch locks, the
  engine turns with the wheels and its inertia is added to theirs, so the wheel-spin
  update stays implicit. The clutch slips when pulling away (and under an engine too
  strong for it) and closes gradually after each shift, during which the throttle is
  cut. Gears are shifted by hand by default; the automatic shifts by rpm and
  throttle. Traction control eases the throttle while a driven wheel spins.
- **Thirteen cars, twelve engine families**: from a city car to a supercar, a
  pickup and a van, each a chassis with a procedurally built low-poly body. Engines
  (a 1.0 three to a 6.5 V12, 22 factory versions) are independent of the cars: the
  showroom (F6) has every car with the engines its factory offers, and the garage
  (F4) fits any engine the engine bay has room for, along with gearboxes, clutches,
  final drives, differentials, suspension, tires and the engine's own parts. Weight
  and balance follow from what's fitted where.
- **Synthesised engine sound** that follows the cylinder layout: each firing sends a
  pulse through its bank's exhaust resonances. A cross-plane V8's banks fire at
  uneven intervals, which gives the burble; a flat four's unequal headers give its
  rumble; the others fire evenly. Two banks are panned apart, and the fuel the engine
  gets makes the pulses louder and harsher.
- **Aerodynamic drag and downforce**, and rolling resistance.
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
- Debug gizmos for the linkage and forces, and a telemetry panel with live-tunable springs, dampers, camber, toe, arm geometry,
  tire parameters and brakes (torque and front/rear split). A HUD in the bottom
  right shows speed and gear inside a rev counter.
- A map of the whole area (K): the ground in its land use colours, drawn once into
  a single image, so it needs no terrain chunks. Roads, houses and the cars are drawn
  on top; drag to move it, scroll to zoom, and click to put the car on the nearest
  road there.
- A driver's view (C) from inside a low-poly cockpit, with a steering wheel that
  turns and a working rev counter and speedometer on the dashboard.

## Running

```sh
nix develop               # toolchain + native libraries
cargo run                 # the sandbox: optimised build (release-dev profile) with fast rebuilds
cargo run --bin career    # the career
cargo test                # physics unit tests and headless driving tests
```

There are two apps built from the same library: the **sandbox** (`src/main.rs`, every
car and part for free, with the developer tools) and the **career** (`src/bin/career.rs`).
Each is the list of plugins it adds to the shared `CorePlugin`.

Or without entering the shell: `nix run`. The web build is served with
`nix run .#serve-wasm` at <http://localhost:8080>. `nix flake check` builds both
targets and runs clippy, rustfmt, nixfmt and deadnix. GitHub Actions runs it on every
pull request, and every push to `main` deploys the web build to GitHub Pages.

## Career

Start in the garage at home with €500 and mom's car (always full and in one piece).
Places are marked on the road with a glowing disc and on the map (K); stop on one to go
in:

- **Pizzeria Bella Strada**: deliver a pizza to a house against the clock; on time and
  without a knock there's a tip.
- **Kessler Cars**: deliver a car to a house. The dealer trusts you more with every clean
  delivery (less with knocks, and when a car comes back undelivered), and trust gets you
  nicer cars that pay more. Afterwards you're back at the dealer with your own car. The
  lot has new and used cars for sale (the car you came in goes home), and the dealer buys
  yours.
- **Motor Parts Thiel**: order parts; they wait on the shelf in your garage, where you
  fit them. Fitting parts never changes the ECU's tune.
- **Gas station**: fill up.
- **Home**: into the garage, a scene of its own: pick the car to take out (A/D, Enter),
  repair its engine, put on new tires, fit parts.

T calls the tow truck, which brings you and your car home (€50 and €2 a kilometre); when
you can't afford it, mom comes. F2 has the driving aids and the wheel settings. The
career is saved in `~/.config/car-sim/career/` (local storage on the web).

## Controls (sandbox)

| Key | Action |
| --- | --- |
| W / S, arrows | Throttle; brake (a stopped car holds on hills) |
| Q / E | Shift down / up (R, N, 1 and up) |
| G | Toggle automatic gearbox (S at a stop then selects reverse) |
| I | Start the engine after a stall (only with a clutch pedal) |
| A / D | Steer |
| Space | Handbrake |
| R | Reset car |
| F1 / F2 | Toggle gizmos / telemetry panel |
| F3 / F4 / F5 | ECU tuning / garage (rebuild the car) / engine dyno |
| F6 | Showroom: pick a car |
| M | Mute engine sound |
| C | Switch between chase and driver's view |
| K | Map of the whole area (drag to move, scroll to zoom, click to put the car on the nearest road there; K or Esc closes) |
| Left-drag, scroll | Orbit and zoom the camera (driver's view: look around) |

### Steering wheel and pedals

Wheels, pedals and shifters work through the system's gamepad support; the keyboard
keeps working alongside. Open the panel (F2), expand "Wheel and pedals", and click
**Bind** next to each control, then move it: turn the wheel left, press a pedal fully
and release it, press a button. Pedals can rest at either end of their axis. The
bindings and settings are saved to `~/.config/car-sim/wheel.cfg` (on the web, in the
browser's local storage). Untick **Use wheel and pedals** at the top of the section to drive
with the keyboard only (the bindings are kept, and force feedback stops).

The wheel steers the front wheels directly at a fixed ratio: either end of the
rotation range is full lock, and the steering wheel in the cockpit turns with yours.
On Linux with the [hid-fanatecff](https://github.com/gotzl/hid-fanatecff) driver,
the game sets Fanatec wheel bases to the range on the slider (540° by default) and
restores their own range on exit. Writing the range needs the driver's udev rules
(your user in the `games` group). If it can't, or on other systems, a base turning
farther than the slider gets a soft lock, like in BeamNG: the steering covers the
slider's range and force feedback holds the wheel at either end. The game reads the
base's range from hid-fanatecff; elsewhere, enter it in the panel (or set the base to
the slider's range by hand). The
paddles shift like Q/E (in automatic mode they select reverse or drive at a stop).
Buttons can also be bound to resetting the car (like R), the camera view (like C) and
the map (like K).

With a clutch pedal bound, you work the clutch: let it out to pull away, and press
it to change gear at once (without pressing it, the paddles still shift with the
automatic clutch). Drop it at low revs or stop in gear and the engine stalls; start
it again with the ignition button or I. Without a clutch pedal, the clutch works by
itself and the engine never stalls. The same goes for the automatic gearbox (G): it
ignores the clutch pedal.

An H-shifter is bound gate by gate (move the lever into each). The gear follows the
lever, and the gearbox is in neutral whenever the lever is between gates; reverse
only goes in near a stop. With a clutch pedal bound, a gear only goes in with the
clutch pressed: move the lever without it and the gearbox stays in neutral until you
press the clutch. Without a clutch pedal, the automatic clutch changes gear. While
an H-shifter is bound and connected, the paddles and Q/E don't shift. The game only knows where a
control is once it has moved: if the lever is already in a gear when the game starts,
it notices at the first move (until then, the gearbox is in neutral).

**Force feedback** (Linux, through the kernel's evdev interface, e.g. with
hid-fanatecff) comes from the front tires: their side force acts a few centimetres
behind the steering axes, pulling the wheel back towards where the car is going.
That trail shrinks as the tires start to slide, so the wheel goes light before the
front lets go. The force starts at half strength, ramps in over a second, and drops
to zero while the window is in the background. It's rate-limited (full force takes
a fifth of a second), so kerbs and crashes don't jolt a direct-drive wheel. Click
**Test: push left** once: if the wheel turns right, tick **Reverse direction**.
Damping and friction come from the wheel base's own tuning menu (on Fanatec bases:
NDP, NFR, INT).

Gizmo colours: yellow for A-arm links, white for uprights, red for coilovers,
green for the raycast and contact point, blue for tire normal force, magenta for
tire grip force, cyan for roll centres.

## Code map

| File | Contents |
| --- | --- |
| `src/physics/rigid_body.rs` | Chassis rigid body |
| `src/physics/suspension.rs` | A-arm linkage, corner dynamics, wheel spin |
| `src/physics/tire.rs` | Magic Formula tire |
| `src/physics/drivetrain.rs` | Clutch, gearbox, drive layouts and differentials |
| `src/physics/engine/` | Engine model, ECU, engine families and their parts, baked maps |
| `src/chassis.rs` | The cars: chassis, gearboxes, and which engine fits where |
| `src/body.rs` | Procedural low-poly car bodies |
| `src/lib.rs`, `src/main.rs`, `src/bin/career.rs` | The shared `CorePlugin`; the sandbox and career apps |
| `src/career/` | Career: places, jobs, the garage scene, dealer, parts shop, towing, save |
| `src/garage.rs` | Sandbox showroom and garage |
| `src/workshop.rs` | Baking engine maps in the background, and their cache |
| `src/sim.rs` | Multi-rate loop, controls to wheel torques, driving tests |
| `src/car.rs` | A car's setup from what it's built of, spawning, keyboard and wheel to controls |
| `src/input.rs` | Wheel, pedal and shifter bindings, config file |
| `src/wheel_base.rs` | Wheel base rotation range and force feedback (Linux) |
| `src/terrain.rs` | Baked map loading, terrain and road meshes, raycast, spawns |
| `src/buildings.rs` | Low-poly houses from the OSM building outlines (visual only) |
| `src/trees.rs` | Trees in forest areas and bushes on scrub (visual only) |
| `src/scenery.rs` | Loads and unloads chunk meshes around the camera |
| `tools/bake` | Separate crate that bakes `assets/*.terrain` from online data |
| `src/visuals.rs` | Bodies, wheels and suspension posed from the simulation |
| `src/debug.rs`, `src/ui.rs`, `src/camera.rs` | Gizmos, telemetry panel, chase and driver cameras |
| `src/hud.rs` | Speed, gear and rev counter HUD |
| `src/map.rs` | Map of the whole area with roads, houses and the cars |
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

Massless arms with a point unsprung mass. No anti-roll bars or tire relaxation
length. Every car has double A-arms all round. Without a clutch pedal the clutch is
automatic, and the engine stays coupled to the driven wheels' average speed. No render interpolation between chassis steps. At very
low speed, slips are computed against a 3 m/s floor, so a parked car can creep
slowly down steep slopes. Bridges become plain embankments. Land use is coloured
per cell (2–8 m), so area edges are stepped, most visibly in coarse chunks.

## Data attribution

- Map data © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, available under the Open Database License (ODbL).
- Elevation: Digitales Geländemodell 1 m (DGM1) © Bayerische
  Vermessungsverwaltung, [geodaten.bayern.de](https://geodaten.bayern.de/opengeodata/),
  licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
