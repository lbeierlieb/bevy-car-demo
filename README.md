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
  coilover spring and damper, anti-roll bar, bump stops). Each axle's anti-roll bar
  pushes its two wheels by how far one is up against the other, so it stiffens roll
  without touching the ride. Whatever part of the hub force doesn't
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
  cut. Gears are shifted by hand (automatic gearboxes are to come); without a clutch
  pedal the clutch works by itself. Traction control eases the throttle while a driven wheel spins.
- **Thirteen cars, twelve engine families**: from a city car to a supercar, a
  pickup and a van, each a chassis with a procedurally built low-poly body. Engines
  (a 1.0 three to a 6.5 V12, 22 factory versions) are independent of the cars: the
  showroom (6) has every car with the engines its factory offers, and the garage
  (4) fits any engine the engine bay has room for, along with gearboxes, clutches,
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
cargo run --bin editor    # the map editor (native only)
cargo test                # physics unit tests and headless driving tests
```

There are two apps built from the same library: the **sandbox** (`src/main.rs`, every
car and part for free, with the developer tools) and the **map editor**
(`src/bin/editor.rs`, no cars; see "The map editor" below). The sandbox is the list of
plugins it adds to the shared `CorePlugin`. On
native, they read the baked map from `assets/gerach.terrain` at startup, so a fresh
bake shows up without recompiling (the copy built in is the fallback, and the web
build's map).

Or without entering the shell: `nix run`. The web build is served with
`nix run .#serve-wasm` at <http://localhost:8080>. `nix flake check` builds both
targets and runs clippy, rustfmt, nixfmt and deadnix. GitHub Actions runs it on every
pull request.

## Multiplayer

The game starts with a menu: **Play alone**, or **Join server** with your name and the
server's address (`host`, `host:port` or `https://host:port`; the port is 4433 unless
said otherwise). On a server each player drives their own car as alone (physics and
force feedback stay on your machine) and sees the others near them (within 600 m), with
their names over the cars; the list of players (top left) has **Leave the server**. A
server is a program of its own, run headless:

```sh
cargo run --bin server -- --cert fullchain.pem --key key.pem   # options: --port, --traffic, --max-players
```

It needs a real certificate for browsers to join (WebTransport). Without `--cert` and
`--key` it makes a throwaway one, which only the native game joins, with
`CAR_SIM_INSECURE=1 cargo run` (for development). The game and the server must be the
same version, on the same map.

### A server on NixOS

The flake has the server as a NixOS module, which gets the certificate from Let's
Encrypt and can serve the web version at the same domain (whose **Join server** then
joins this server):

```nix
{
  inputs.car-sim.url = "github:<you>/bevy-car-demo";

  outputs = { nixpkgs, car-sim, ... }: {
    nixosConfigurations.vps = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        car-sim.nixosModules.server
        {
          security.acme = { acceptTerms = true; defaults.email = "you@example.org"; };
          services.car-sim = {
            enable = true;
            domain = "cars.example.org";  # a DNS record pointing at the machine
            openFirewall = true;          # UDP 4433, TCP 80 (and 443 with the web version)
            web.enable = true;            # https://cars.example.org: the game in the browser
            # port = 4433; traffic = 20; maxPlayers = 32;
          };
        }
      ];
    };
  };
}
```

The server runs as `car-sim.service` (`journalctl -u car-sim` shows who joins and
leaves) and takes up renewed certificates by itself, without dropping anyone. The flake
also has the packages alone: `nix build .#server` and `nix build .#web`.

## Controls (sandbox)

The game starts with a screen listing the controls (a click or Esc closes it, H brings
it back); the panel and the tool windows are hidden until their keys open them. When
the car won't do what you seem to want, a hint over the road says why and what to
press: giving gas with the parking brake on or in neutral, asking for reverse while
rolling, bouncing off the rev limiter below top gear, a stalled or wrecked engine, an
empty tank, or a few seconds of gas without moving (stuck: R).

| Key | Action |
| --- | --- |
| W / S, arrows | Throttle; brake |
| Q / E | Shift down / up (R, N, 1 and up; reverse only at a standstill) |
| I | Start the engine after a stall (only with a clutch pedal) |
| A / D | Steer |
| Space | Parking brake on/off (cars start with it on) |
| R | Put the car on the nearest road (in its lane), as new: repaired, refuelled, fresh tires |
| H | The controls screen |
| 1 / 2 | Toggle gizmos / telemetry and settings panel |
| 3 / 4 / 5 | ECU tuning / garage (rebuild the car) / engine dyno |
| 6 | Showroom: pick a car |
| F11 | Fullscreen (the browser's on the web; all F keys go to the browser there) |
| M | Mute engine sound |
| C | Switch between chase and driver's view |
| K | Map of the whole area (drag to move, scroll to zoom, click to put the car on the nearest road there; K or Esc closes) |
| Left-drag, scroll | Orbit and zoom the camera (driver's view: look around) |

### Steering wheel and pedals

Wheels, pedals and shifters work through the system's gamepad support; the keyboard
keeps working alongside. Open the panel (2), expand "Wheel and pedals", and click
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
the slider's range by hand). The paddles shift like Q/E. Buttons can also be bound to resetting the car (like R), the camera view (like C) and
the map (like K).

With a clutch pedal bound, you work the clutch: let it out to pull away, and press
it to change gear at once (without pressing it, the paddles still shift with the
automatic clutch). Drop it at low revs or stop in gear and the engine stalls; start
it again with the ignition button or I. Without a clutch pedal, the clutch works by
itself and the engine never stalls.

An H-shifter is bound gate by gate (move the lever into each). The gear follows the
lever, and the gearbox is in neutral whenever the lever is between gates; reverse
only goes in near a stop. With a clutch pedal bound, a gear only goes in with the
clutch pressed: move the lever without it and the gearbox stays in neutral until you
press the clutch. Without a clutch pedal, the automatic clutch changes gear. While
an H-shifter is bound and connected, the paddles and Q/E don't shift. The game only knows where a
control is once it has moved: if the lever is already in a gear when the game starts,
it notices at the first move (until then, the gearbox is in neutral).

**Force feedback** (Linux, through the kernel's evdev interface, e.g. with
hid-fanatecff) is what the front wheels do to the steering, worked out from the
suspension as it stands: their side force acts a few centimetres behind the steering
axes, pulling the wheel back towards where the car is going. That trail shrinks as the
tires start to slide, so the wheel goes light before the front lets go. On top come
braking and bumps through the scrub radius, the load through the inclined axes, and
with front- or all-wheel drive the driveshafts' torque: it tugs over bumps under
power, straightens the wheel as the body rolls, and pulls a little to one side behind
a transverse engine, whose shafts differ in length. Full force is about what the
car's front tires reach near the limit, so a light car's wheel is as alive as a heavy
one's. **Road feel** sets how strongly the quick part (bumps, tugs) comes through
against the steady pull (1: as the tires make it; 2.5 to start with). The force
starts at half strength, ramps in over a second, and drops
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
| `src/lib.rs`, `src/main.rs` | The shared `WorldPlugin` and `CorePlugin`; the sandbox app |
| `src/net/`, `src/online.rs`, `src/bin/server.rs` | Multiplayer: protocol, server, client, WebTransport; the start menu; the server app |
| `src/editor/`, `src/bin/editor.rs` | The map editor |
| `src/osm.rs` | The map file's reader and writer (shared with the bake) |
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
| `src/help.rs` | The controls screen and the hints for beginners |
| `src/map.rs` | Map of the whole area with roads, houses and the cars |
| `src/cockpit.rs` | Interior for the driver's view: dashboard, steering wheel, gauges |

## The map and baking it

The map's roads, land use and buildings live in `map/gerach.osm`, an OpenStreetMap
file that is ours: it was imported from OpenStreetMap once and is edited from then on
in the map editor (below) or [JOSM](https://josm.openstreetmap.de/) (File → Open,
edit, File → Save). The file
says `upload='never'`, so JOSM won't upload it to OpenStreetMap. Elements keep their
OSM ids; JOSM gives new ones negative ids and keeps deleted ones in the file, marked
`action='delete'`, so a later import doesn't bring them back. The bake only reads the
tags it knows (see `road_style` and `land_class` in `tools/bake/src/main.rs`).

`tools/bake` has two commands. `import` adds what OpenStreetMap has in and around a
map rectangle to the map file (making it if needed), through a single Overpass API
query, keeping only the tags the bake might use. Whatever is already in the file stays
as it was edited, so a new area can be added next to an edited one. The bake reads the
map file and the lidar terrain model (DGM1, 1 km tiles) from the Bavarian surveying
administration's open data server. Downloads are cached in `tools/bake/cache`
(delete it to fetch fresh data), so re-baking doesn't load the servers again. The map
frame is aligned with UTM zone 32, like the lidar tiles. Elevation is box-filtered
onto a 2 m grid and land use is rasterised per cell.

Roads are their own geometry (`tools/bake/src/roads.rs`). Each centreline is a
Catmull-Rom spline through the mapped points, sampled every metre, so roads still
meet where they share a point. Heights along all roads are solved at once as least
squares (`profile.rs`): close to the lidar (the median across the road's width, so a
centreline mapped onto a verge still finds the road), with changes of slope costing
enough to smooth out bumps shorter than 80 m on main roads down to 25 m on tracks.
Roads share their height where they meet and bend smoothly where one runs on into
another, and lidar heights far off the profile count less. Bridges get next to no
lidar weight, since lidar shows the ground under them, so their decks span smoothly
between the approaches, over the ground (lowered to below the slab where it would
come up through it). The game draws them as a slab with
parapets (which cars collide with), and its ground query has levels: something at
height y stands on the highest surface at most a metre above it, so cars on a deck
stay on it and roads under it pass beneath. The surface has a crown
(2.5 % on paved roads, 3 % on tracks), and the terrain is cut and filled to it: 8 cm
under the road and a metre of shoulder, then sloping at 0.6 to the natural ground.
In the game, a road's surface (blended where roads overlap, so junctions have no
steps) is both what is drawn and what the tires drive on.

Roads have small bumps on top (`src/relief.rs`), as uneven as their `smoothness` tag
says: `excellent`, `good` (ISO 8608 class A), `intermediate` (B), `bad` (C),
`very_bad` (D), `horrible` (E). Untagged asphalt is `good`, stones `intermediate`,
gravel between `intermediate` and `bad`, dirt between `bad` and `very_bad`. To make
an older stretch of road, split the way in JOSM and tag that part. The bumps are a sum
of waves 0.5 to 20 m long with ISO 8608's spectrum, fading out towards the edges. A
node on (or next to) a road tagged `traffic_calming=bump` (0.6 m long, 8 cm high),
`hump` (3.7 m, 7 cm) or `table` (8 m with 1 m ramps, 7 cm), or `hazard=pothole` (0.8 m
across, 6 cm deep), builds that into the road; `height`, `diameter` and `depth` in
metres change the sizes.

Mapped centrelines are often a metre or two off, and road widths are guesses where
`width` isn't tagged, so roads ran into houses. The bake moves each road sideways off
the building outlines (`tools/bake/src/clearance.rs`), by up to 2 m, tapering
smoothly along it, until its edges clear the walls by 30 cm. Points shared with
other roads stay put, so roads still meet. Bridges and tunnels aren't moved. On the
current map this moves 217 roads and clears 197 of the 303 overlaps. What's left
is written next to the map file as `map/gerach.conflicts.gpx` (not committed),
deepest first. Open it in JOSM (File → Open) as a layer of markers beside the map,
then move the way or tag a narrower `width`. Each marker says the way and the
building, how far the road reaches in, the room between the houses where that's
narrower than the road, and whether it's at a junction (where the road can't move).

Fences come from `barrier` ways (`tools/bake/src/fences.rs`, drawn and made solid
in `src/fences.rs`). `barrier=fence` is wooden posts and rails, or thin metal ones for
`fence_type=chain_link`, `metal`, `railing`, `wire` and the like, or closed boards
for `panel`, `concrete` and `palisade`. `wall`, `hedge`, `guard_rail`, `railing` and
`jersey_barrier` are built too; `height` in metres overrides the default. A
`barrier=gate` (or `lift_gate`, `swing_gate`, `entrance`, `cattle_grid`…) node on
a barrier opens a gap 3.5 m wide, or `width` metres; gates are always open.
Barriers running along a road are moved out to 50 cm beyond its edge (mapped guard
rails are often a metre off, and road widths are guesses), and those along a
bridge are left out (its parapets stand in for them). Barriers still standing on a
road, mostly ones crossing it without a gate, are listed in the conflicts file too.
Every barrier is solid: cars stop at it like at a wall.

Two kinds of edits are ours, made in the map editor. `ele` on a node (metres above
sea level) pins the height of the roads through it: the profile passes through it
exactly and bends smoothly towards it. A closed way tagged `carsim:terrain` changes
the lidar terrain before the roads are fitted (`tools/bake/src/terrain.rs`):
`flatten` to `carsim:height` (above sea level), `raise` by `carsim:offset`
(negative lowers), `smooth` over `carsim:radius` metres (default 6), or `slope`, a
plane through the `ele` heights of its corners. Each fades in over `carsim:fade`
metres (default 5) inside its outline; its chunks get full detail.

Each 96 m chunk is then given a resolution. It gets 2 m within 20 m of a building
or 14 m of a road, and next to paved areas. Everything else gets 8 m, with each coarse vertex averaging the terrain around it. Along seams
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
./target/release/bake import <lat> <lon> <width_m>[x<height_m>] ../../map/<name>.osm
./target/release/bake ../../map/<name>.osm <lat> <lon> <width_m>[x<height_m>] \
    ../../assets/<name>.terrain [<spawn_lat> <spawn_lon> [<spawn_heading>]]
```

The map was imported with `import 50.0118569 10.8455963 9600x8448 ../../map/gerach.osm`
and is baked with `../../map/gerach.osm 50.0118569 10.8455963 9600x8448
../../assets/gerach.terrain 50.0305556 10.8046389 180` (about 20 seconds; the map
editor runs the same with F5), which starts the cars on the road nearest that point in
Gerach, facing as close to south (180°) as the road allows, one behind the other. To use a different file,
change `BAKED` and `TERRAIN_FILE` in `src/terrain.rs` (and the map file and bake
arguments at the top of `src/editor/mod.rs`). To make the map bigger, import the bigger
rectangle into the same file and bake with the new size. The public Overpass server sometimes answers
that it's busy; the tool then retries every minute, up to five times. Only chunks near the camera are on the GPU, so map size is limited by
the size of the baked file, which is embedded in the program (zlib-compressed, with heights and road points as small differences: about 0.19 MB per km²
with the current mix of resolutions).

## The map editor

`cargo run --bin editor` (in `nix develop`, from the repo) flies over the baked map
with the map file drawn on top, coloured by kind: roads yellow, waterways blue,
buildings red, barriers purple, land use green, terrain areas orange. It edits
`map/gerach.osm` directly, written the way JOSM writes it (new elements with negative
ids, changed ones marked `action='modify'`, deleted ones kept as `action='delete'`),
so JOSM and `bake import` still work with the file.

- **Flying**: hold the right mouse button to look around, WASD to move, Q/E down and
  up, Shift faster, the mouse wheel sets the speed. M opens the map; a click there
  flies to that spot.
- **Selecting** (tool 1): click a way or a node; the panel shows its id and tags, as
  rows of key and value (typed values are written when the field is left, one undo
  step; `…` offers values for keys like `highway`, `surface`, `smoothness`), with
  buttons adding common keys. Drag a node to move it, Ctrl+click on a way to add a
  node there, Delete removes the selected node (or way), X splits the selected way at
  the selected node.
- **Drawing** (tools 2–6: road, fence, building, land use, terrain area; the kind is
  picked in the panel): click to add points. Clicking a node joins it, clicking a way
  adds a shared node to it. Enter or a click on the last point finishes; a click on the
  first point closes an area. Backspace takes a point back, Escape cancels. A new
  terrain area gets the terrain's mean height (flatten), 1 m (raise), a 6 m radius
  (smooth) or its corners' heights (slope), all editable in the tag table.
- **Placing** (tool 7): speed bumps, humps, tables and potholes on a road (or where
  clicked), gates on a fence, and height pins on a road.
- **Heights**: Page Up/Down raise and lower the selected node's height pin by 10 cm
  (Shift: 1 m); it starts at the ground there. The pin shows as a cyan post to its
  height.
- **Conflicts**: the bake's list (`map/gerach.conflicts.gpx`) is shown as red posts with
  labels; Previous/Next fly to them, and buttons select the way and building one names.
- **File**: Ctrl+Z / Ctrl+Y undo and redo, Ctrl+S saves. F5 (or "Save and bake")
  saves, builds and runs the bake (about 20 s, progress in the panel) into
  `assets/gerach.terrain` and reloads the map in place.

## Deliberate simplifications

Massless arms with a point unsprung mass. No anti-roll bars or tire relaxation
length. Every car has double A-arms all round. Without a clutch pedal the clutch is
automatic, and the engine stays coupled to the driven wheels' average speed. No render interpolation between chassis steps. At very
low speed, slips are computed against a 3 m/s floor, so a parked car can creep
slowly down steep slopes. Bridges have no piers. Land use is coloured
per cell (2–8 m), so area edges are stepped, most visibly in coarse chunks.

## Data attribution

- Map data © [OpenStreetMap](https://www.openstreetmap.org/copyright)
  contributors, available under the Open Database License (ODbL). `map/*.osm` is
  derived from it (with our edits) and is available under the ODbL too.
- Elevation: Digitales Geländemodell 1 m (DGM1) © Bayerische
  Vermessungsverwaltung, [geodaten.bayern.de](https://geodaten.bayern.de/opengeodata/),
  licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
