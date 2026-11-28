---
name: screenshots
description: How to render headless screenshots of the game, the editors, shops, scenes and two-player sessions with tools/dev/shot.sh (env vars, places, taps, cameras). Use before taking any screenshot or checking anything visually.
---

# Screenshots (`tools/dev`)

`tools/dev/unstrip.sh` copies `tools/dev/shot.rs` into `src/` and hooks it into
`lib.rs` and the apps; `tools/dev/shot.sh out.png [frames] [keys]` renders headless and
saves a screenshot (`BIN=editor` for the editor); `tools/dev/strip.sh` removes the hook.
Always run `strip.sh` and check that `grep -cw shot src/lib.rs src/main.rs
src/bin/*.rs` prints 0 for each before committing. Keys are pressed from a third of
the frames until the shot (`m` opens the map, `2` shows the telemetry panel, `f` shows the gizmos (off by default), `c` the driver's view, `b` looks back; keys stay held through the shot's frame),
so clicks on a window must come after that (`T` presses the key T, `E` Enter).
Shots play alone (on the game's own server, with the repo's shops and events) unless
`JOIN`, `MENU=1`, `OFFLINE=1` (no server, no traffic; the shot spawns the built-in
catalogue's first car itself, as no server gives one) or `LINEUP` (likewise, as a join
would move the car). `PLACE` works because `shot.sh` sets `CAR_SIM_TRUST_PLACEMENTS=1`
(the game's own server then takes up placements however far they move the car).
Shops need `PLACE` at the building's point
(the car lands on the marker and goes in): gas `-2603,-1736`, car dealer
`-1476,-1213`, alignment `-1480,-1305`, paint `3500,-63`, car workshop `-1566,-1329`,
tuning shop `-1950,-3160`, tire shop `-2502,-1763`, engine shop `120,2700`, dyno
`-2601,-3784`, garage park `-2680,-1793`. Since 2026-11-27 the car stopped on a marker
only offers the shop ("Press Enter to go into …"): **put `Enter@190;` before every
shop's `TAPS` in these notes** (frames after it as they are), which opens the shop's
window (the focus on its first row, so `TAPS="Enter@190;Enter@200"` goes into the scene: onto the rollers, up to the pump…; older click
coordinates in these notes are from before the menus, 2026-11-23, and are wrong now;
the car hunt's marker `-95,2341`, its lobby window opens
at once; a race marker exactly (the repo list has none since 2026-11-24: add one with a
route; "Races Baunach" was at (−62, 2280)): `PLACE_EXACT=1 PLACE="-62,2280,150"`, then
`TAPS="Enter@200"` is "Time trial", `ArrowDown@200;Enter@204` "Race"; in the dyno's room
the page is the first row (`ArrowRight` steps it: runs, ECU tune, cams and boost), "Hold"
the third; a hold needs ~200 frames more to settle).
The editor (`BIN=editor`, hooked by `EditorShotPlugin`): `EDITCAM="x,z,height,yaw,pitch"`
(degrees, the first 20 frames), `CLICK="x,y,frame[,x2,y2]"` sets the window's cursor and
holds the left button (a drag with x2, y2), `UICLICK` is the window-event click for
egui (the map, buttons), `EDITKEYS="Digit2@25;ControlLeft+KeyS@60"` taps keys. A view
of a village road: `EDITCAM="-2986,-1954,45,0,-70"`, the road at (1000, 381). Saving
or baking in a shot writes the real map and terrain files: `git checkout map assets`.
`PLACE="x,z,frame"` puts the driven car on the road nearest that map point (joined, at frame 150 or later, or the join moves it: e.g. by a shop, `JOIN=127.0.0.1 NAME=Anna PLACE="-1476,-1180,150"`, 220 frames). Give a shot
its own config dir (`XDG_CONFIG_HOME=<scratch>/cfg`) for fresh settings. The start menu
is closed in shots unless `MENU=1`, the quick start before it unless `INTRO=1`; `JOIN=127.0.0.1 NAME=Anna` joins a server at once.
Two players: start the server (`target/sandbox/release-dev/server`, throwaway
certificate, in the background with `nohup`), then two shots with `CAR_SIM_INSECURE=1`
and their own config dirs, the second started a few seconds after the first, more
frames for the first (they take turns building). Don't `pkill -f` anything matching
your own command line (it kills the shell); `pgrep -x server`, then `kill`.
Likewise, never wait for a shot with `until … ! pgrep -f shot.sh` in a background
shell: the pattern matches the loop's own command line, so it never ends (three such
loops ran for hours, 2026-11-22). Wait for the output file, or `pgrep -x`/a PID.
`CLICK="x,y,frame;x,y,frame…"` clicks into the
1280×720 window, e.g. on the map. `CAM`/`CAMAT` env vars place the camera (see
`shot.rs`). `shot.sh` fetches Mesa and Xvfb from the pinned nixpkgs on first use.
A shot takes about 35 s.

A mid-engined car: `OFFLINE=1 TRIM=2,1 FITIDS=tessel.rear-subframe.engine` (fitted as a
shop does, the engine moving along); into the bay `UNFIT=Hatch CARCAM="-1.4,2.5,-3.4,-1.0"`.
For looking at cars: `CAR="chassis[,engine]"` starts in that car (its place in the
catalogue; bought from the server's showroom, `Buy`); `LINEUP="first,count"` parks that many cars one behind the other on an open,
flat straight (the spawn is in a village, where walls and slopes block the view);
`CARCAM="side,height,along[,ahead]"` looks at them (or `ahead` m ahead of their middle) from that far to their right (negative:
left, the sunlit side), up, and ahead. Side views of five:
`LINEUP=0,5 CARCAM="-21,2,0" bash tools/dev/shot.sh out.png 60 2`; one car close up:
`LINEUP=3,1 CARCAM="-3.6,1.9,4.6"`; from the driver's seat: keys `2c`.
`TRAFFICCAM="back,height[,nth]"` follows the nth nearest traffic car from behind, e.g.
`TRAFFICCAM="9,2.2,1" bash tools/dev/shot.sh out.png 200 2`.
`PROXYCAM="distance,height,bearing"` looks at the other player's car (a named remote car) from that far
towards the bearing (degrees from north). Two players side by side (the second needs
`PLACE` after it has joined, or the join puts it elsewhere): Anna with
`PLACE="-2986,-1954,150"` and many frames, Ben with `PLACE="-2986,-1975,150"` sees her
ahead. Egui headers can't be clicked open in shots (change `default_open` for a shot).
