# Notes: Events, races and the race editor

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/events/`; keep it up to date.

## Events (agreed with the user, 2026-11-22; built the same day)

Like shops, places on the map that the server lists (`server --events FILE`,
`services.car-sim.events`; repo list `assets/events.cfg`, lines `kind x z name`, names
made up). Unlike a shop's, an event's marker is exactly at its point, not moved to a road
(the user wanted the hunt in the middle of a parking lot, 2026-11-22; the server checks
"near the marker" against the point too). Each round of an event runs in a **world of its own** (see "Parallel worlds"):
its players see no cars, traffic or shops of the normal world and the normal world sees
none of theirs; traffic, if on, is simulated for that world alone; the chat is per world.
Another group can start an independent round at the same marker while one runs. The
players list says who's in which event. Every game runs on the same frame
(`round.rs`: `Round { event, players, host, game, phase, best }`, lobby → running →
result; `Game`, `Play`, `Outcome` are enums per game; the host is the hunt's hunted or
whoever opened a race's lobby).

**Car Hunt** (first event, "Car Hunt Baunach" in the middle of the Rewe's parking lot in
Baunach, (−87, 2286), the `amenity=parking` area's centroid):

- Stopping on the marker joins the lobby forming there (or opens one). The first one in
  is the hunted, all later ones hunters. The hunted can click another name to make them
  the hunted, sets the options and starts (at least one hunter). No late joining.
- Options (the hunted's): head start (10 s), time until the hunted wins (10 min),
  traffic (on), "hunters can reset" (on; R calls the rescue
  helicopter, whose time is the penalty for crashing; "to the start" was dropped at the
  user's wish, 2026-11-22; the hunted can never reset), cars ("player cars" (default) or "factory
  cars": everyone is lent the start car, factory-built, full and on fresh tires (was
  "muscle vs rally" until the other chassis went, 2026-11-26); a lent car leaves no
  trace, players get their own car back afterwards as it was), how the hunters win
  (`Catch`: "Tag them" (default) or "Stop them"; added 2026-11-22, the user: tagging is
  the objective, with one or two hunters stopping the hunted is near impossible).
- Everyone starts near the marker; hunters are held for the head start. Name tags on and
  everyone in the round on the map, whatever the server's settings.
- Hunters win when the hunted's car is under 10 km/h for 10 s, anywhere (after the head
  start), or, with "Tag them", the moment a hunter's car touches the hunted's (or the
  other way round); the hunted wins when the time runs out. As built: each game's
  `push_off_remote_cars` collects the remote cars ours touched (`net::Touching`), the lobby
  sends them in a running tag round (`Action::Touched(CarId)`, at most every 0.2 s), and
  the server ends the round (`Ending::Tagged { by }`) if the car's owner and the sender are
  the hunted and a hunter, past the head start, their states within `TAG_RANGE` (15 m).
  Ghosts touch nothing, so cars that waited overlapping in the lobby tag only once apart.
- The hunted sees the nearest hunter's distance in the bar (from `Connection::on_map`,
  1 Hz, carried on at speed; rounded to 5/10 m, yellow under 200 m, red under 50 m). If the hunted leaves, the round ends.
- Afterwards everyone is back in the normal world at the marker, in the lobby menu (the
  next round's lobby there, if any). Fuel, tires and wear of a player's own car carry over.

As built: `traffic::Realm` (component; none = `Realm::NORMAL`) on peers and traffic cars;
`Traffic::realms` overrides the density per realm; `populate` and `step` work per realm;
the server's send loop, map and chat only within a realm, and in any realm but the normal
one tags are on and the map shows everyone. `src/events/`: `mod.rs` (list, `parse`,
markers, map marks: `MapMarks`/`MapLegend` are keyed by owner now), `hunt.rs` (rules,
`Settings`, `Outcome`, their wire), `round.rs` (`Round`, `Phase`, `Action`), `rounds.rs` (server:
`Rounds`, `run_rounds` in `ServerPlugin` between `receive` and `send`; gives and takes
only the realms it manages; one lobby per event, a round over merges into the waiting
one), `lobby.rs` (client: `arrive` asks to enter when stopped on a marker, drive 8 m or
Esc leaves; `follow` places the car at `online::lane_spots` near the marker by slot
(a lent car and our own back come from the server as `YourCar`), sets `Car::hold` and
`car::ResetRule`; `window`: lobby window and the bar at the top). Protocol 7:
`ToClient::Events`, `Round(Option<Round>)`, `Busy`, `ToServer::Event(Action)`; protocol 8
added `Settings::catch`, `Ending::Tagged`, `Action::Touched`. Times are
on the server's clock (`Connection::server_time`). Shops and event markers are hidden
in a running round (`Connection::in_running_round`). Tests: `realms_see_only_themselves`,
`a_car_hunt_runs_in_a_realm_of_its_own`, `a_hunt_lends_cars_and_gives_them_back`,
`a_hunter_tags_the_hunted` (test apps have no shops plugin, so `WantsGhost` stays set
from the lobby unless a test clears it).
Not done: a round's own chat lines (who caught whom) for the server's chat.

**Races and time trials** (asked for by the user, 2026-11-22; built the same day;
grid and gates by hand since 2026-11-24, the user: the automatic gates in Baunach
weren't great): a `race` event (blue marker) has routes listed under it in the events
file, made in the race editor: `route loop|sprint NAME`, then `grid x z heading` lines
(compass degrees; up to 16; pole = nearest the first checkpoint, `race::order_grid`)
and `x z` lines, its checkpoints, **each a gate** (several routes per event; needn't
start at the marker; `events::read_routes` reads an export without a race line).
`events::resolve_route` (the server at startup, `Terrain::load`; the race editor live)
finds the way along **every** road but streams (`routes.rs`: a graph of stretches
between junctions like `traffic::network`, Dijkstra between consecutive checkpoints,
unpaved ×3 and under 3.5 m ×1.5 per metre), refusing a checkpoint over 30 m from a
road, a route turning back at one (way out against way in, cosine < −0.7 over 15 m),
shorter than 500 m, or without a grid; the line (simplified to 0.3 m, the checkpoints'
points kept), widths and `marks` (each checkpoint's index in the line) go out with
`ToClient::Events` (protocol 16). The repo's Baunach routes were removed (2026-11-24;
"Races Baunach" with them, as a race needs a route): the user makes new ones. The old
"Baunacher Ring" points are the `RING` test route in `events/mod.rs`. Agreed with the
user:
- On the marker a window asks: time trial (`Action::Trial`, a private round of one,
  `Game::Trial`, never merged or joined) or race (`Action::Enter`: the lobby there, the
  first in hosts and sets things; at most the route's grid spots; starts with 2+, not
  with more racers than the picked route's spots).
- Settings (`race::Settings`): route, laps (loops, 1–20, default 3; no reverse since
  the grid is by hand: a reverse race is a route of its own),
  traffic (off), cars collide (on; off: `net::GhostPlayers`, the client treats remote
  cars with a name tag (in a realm, players' cars) as ghosts, traffic still collides),
  racers can reset (on; R: `ResetRule::Spot`, the rescue helicopter to the last gate
  passed, right half of the road; time trials never reset, "Start again"
  (`Action::Restart`) instead). Player cars only, for now.
- Running (`race.rs`: `Course` = a route with laps): each racer on their grid spot by
  slot (R before any gate goes back there), held (`Car::hold`) for `COUNTDOWN` (5 s),
  the parking brake off, in neutral (put in gear during the countdown; agreed with the
  user, 2026-11-22); the clock runs from the green light. Gates
  (`race::checkpoint_gates`): at each checkpoint's mark, facing the line's way in and
  out over 10 m averaged; a sprint's are all checkpoints (the last the finish), a loop's
  lap is the second on, then the first (passing the first from the grid is the start,
  not a gate; lap 1 includes the way there); the server judges them from the cars'
  states (`Gate::crossed`: forwards, within road half-width + 6 m, not a jump over 60 m),
  in order. "Wrong way!" only within 30 m of the line (not on the way from the grid). Standing (`Racer`:
  gates, last gate's time, laps, finished, left) goes out with the round on every gate.
  First one through: the rest have `FINISH_GRACE` (60 s); over when all are through or
  gone; results (`race::Results`) in the lobby. Leaving mid-race lists you as "Left".
- Best times: per (event, route, traffic), time trials only, a loop's laps or
  a sprint's whole way, top 10, one per name, for the session (`Rounds::best`, sent as
  `Round::best`).
- Client (`racing.rs`): the chooser, the race lobby window, the bar (countdown, lap,
  place, clock, lap times, gap to the one ahead from `EventState::gate_times`, a painted
  arrow and distance to the next gate (the font has no arrow glyphs), "Wrong way!"),
  gates on the road (two posts and a banner: next green, the two after white, the finish
  yellow), the route on the map (`map::MapLines`) and the next gate as a mark.
Tests: `a_race_runs_from_the_grid_to_the_results`, `a_time_trial_puts_its_lap_on_the_best_times`,
`the_races_routes_are_found_along_the_roads`, `race::tests`.
Not done: race-specific cars ("everyone the same car"), saving best times, rolling
starts, a minimap.

**Race editor** (asked for by the user, 2026-11-24, instead of recording a route while
driving; `src/race_editor/`, `src/bin/races.rs`, `cargo run --bin races [FILE]`): the
map editor's fly camera (`editor::camera`, 3× as fast: `SpeedScale`; Shift+wheel no longer sets its speed) and
ground hit (`editor::overlay::ground_hit`), no cars. `mod.rs`: `Draft` (the `Plan`:
looped, grid in placement order, checkpoints; undo/redo of whole plans, `touch` while
dragging/turning and `commit` at the end, `change_merged` for wheel notches; the route
found again at most every 0.1 s by `resolve_route` on a `RoadGraph` built at startup;
`text` = the export: comment, commented `race` line at pole, `Route::text`). `edit.rs`
(G grid tool: click places a box of the biggest car's body box, `show::CarBox`, turning
to face the pointer until the next click, Esc calls it off; click on a spot's disc or
box turns it again; Shift+wheel 1°; C checkpoint tool: click appends, Ctrl+click
inserts into the nearest leg of the found line (straight legs while none), a click on
the first checkpoint's disc loops; discs drag (4 px makes a drag); Delete removes what's
under the pointer; Ctrl+Z/Y). `show.rs` (boxes red where they overlap, arrows, discs,
gates via `racing::spawn_gate` (start green, finish yellow), the line as gizmos, red
straight lines while not found, egui labels, map lines and marks). `panel.rs` (window;
Open and Export dialogs asking a file name in the working directory, `.cfg` added,
asking before overwriting). Shots: `BIN=races` (unstrip hooks `EditorShotPlugin`; keys
`KeyG`/`KeyC` in `EDITKEYS`); a road at a village: `EDITCAM="-2965,-1972,28,-153,-62"`
runs up the screen through x 600–660 (`CLICK="600,520,30;600,380,40"` places a spot
facing up it). Nothing of it tried with a real mouse yet.
