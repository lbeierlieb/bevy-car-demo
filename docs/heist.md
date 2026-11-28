# Heists (planned with the user, 2026-11-28)

The user asked for a new event: pull up to a bank (a disc, like any event's marker), see
the robbery in a scene of its own, pick how much money to take, then drive in a world of
your own to an escape point where a helicopter escort waits. It flies you away and puts
you in your garage. The more you take, the more police there are and the harder they
chase. If they stop you before the escape point, they impound your car and you are put
in your garage.

## What it builds on

- **Events** (`src/events/`): a marker from the server's list, a `Round` per group in a
  realm of its own (`traffic::Realm`), lobby → running → result, settings per `Game`.
  A time trial is a private round of one (`Game::Trial`), which a heist copies.
- **The server moves cars** (`Shadow::put`, `events::start_spot`/`marker_spot`) and
  **carries them** (`heli::rescue`: `Rescue::call` to a spot, `special_rescue` already
  carries a car to a far shop in at most `LONGEST_CARRY`, 30 s, as a scripted car).
- **The hunt's helicopter** (`heli::Chase`) follows a car from the air on the server.
- **Money** (`trade.rs`, `Peer::pay`): wallets per account, guests for the session.
- **The garage park** (`garage/park.rs`, `owned.rs`, `net/stash.rs`): garage cars,
  `Stash::Drive`, the hall scene 250 m under the marker.
- **Traffic** (`traffic.rs`): kinematic drivers along the road network, simulated only
  on the server, per realm. They crash on any knock and then tumble as loose bodies.
- **Scenes under markers** (gas station, paint booth, workshop, showroom): the model to
  copy for the bank.

There are no police, no people in any scene, and no way to be without a driven car.

## Assumptions

The user confirmed 2, 3, 11 and the bank scene (answers 2026-11-28) and changed 4; the
rest are mine, unchallenged.

1. **Solo.** A heist is a private round of one, like a time trial. Crew heists (a
   getaway driver plus others) come later, if wanted.
2. **Your own car is the stake.** No lent cars. Being busted takes the car you drove,
   with its trunk.
3. **Impound** means the car goes to a new place, `impound`, in the database. It isn't
   destroyed. You get it back at an impound lot (a new shop kind) for a fine, built in
   the last milestone. Until then an impounded car is gone for good. (The user: "Losing is
   the stake I want.")
4. **After a bust** you are put in the garage park's hall **without a car** (the user's
   answer). You pick one of your garage cars to drive. With an empty garage the hall
   offers **"Get a free car"**: a scene where your parents give you a car (the server's
   start car). No people in it, as the game has none: the car on your parents' drive with
   a big bow on it and a card, "From Mum and Dad". Being without a car is a new state:
   the game must cope with no driven car while in the hall (no HUD, no driving, no
   prediction), and the server with a peer that has none (sent nothing, simulated
   nothing, no traffic anchor). "Get a free car" is offered only with no car at all, so
   it can't be farmed.
5. **Leaving counts as a bust.** That covers Esc or "Leave", a disconnect, and the
   server dropping a silent player. Otherwise leaving would be a free way out.
6. **No R.** During a heist the reset rule is `ResetRule::Never`, as for the hunted. If
   the rescue helicopter worked here, it would be a second getaway. A stuck or stranded
   car gets busted.
7. **Loot** is picked in steps from 2 000 € to 250 000 € (`trade::price_steps`-like).
   You are paid only when the escort sets you down. The amount picks a **heat level**,
   1 to 5, which sets the police.
8. **Escape points** are listed under each bank in the events file (`escape x z` lines).
   The server picks one at random, the higher the heat the farther one. It shows on the
   map and as a marker with an arrow and the distance in the bar, like a race's next gate.
9. **The getaway** means stopping (under 10 km/h) within 8 m of the escape point. From
   then on the server has the car (scripted, as in a rescue), so the police can't bust
   it. The escort hooks it, lifts it off, and carries it to the garage park (the
   `special_rescue` carry, capped at 30 s). You land in the hall, with the loot paid.
10. **A bust** happens when your car is under 10 km/h for 5 s with a police car within
    12 m. The bar shows a meter filling. It mirrors the hunt's "Stop them". A wrecked
    or stalled car also counts if police are that near.
11. **Police are kinematic, like traffic, not simulated cars** (the user: try simple
    scripted ones for now and see how it feels). A simulated car costs
    the VPS about 10 % of its vCPU, which rules out up to ten police cars. A pursuit
    driver routes along the road network towards you, goes over the limit, and within
    ~40 m leaves the lane to aim at you and block you. A knock only ends its drive past
    a set impulse (ramming it hard puts it out). Your car pushes itself off them as off
    traffic, so being rammed pushes you.
12. **The police look made up**: one of the catalogue's chassis in a generic livery
    (white with a stripe, "POLICE" on the sides, no real force's colours or crest), a
    lightbar flashing blue (and lit at night via `night::Glow`).
13. **Cooldown**: one heist per player per 10 minutes, on the server's clock, for the
    session only.
14. **Traffic** in the heist's realm is on, at the server's normal density.
15. **The bank** is in Reckendorf, at the real Raiffeisen bank's building (the user's
    pick; its point from the map's OSM data in milestone 1), under a made-up name, never
    the real one (e.g. "Kreditkasse Reckendorf"). Three escape points 1–4 km away.

## Heat levels (first numbers, to tune by playing)

| Heat | Loot up to | Police cars | They join after | Top speed | Extras |
|---|---|---|---|---|---|
| 1 | 10 000 € | 2 | 25 s | limit + 20 % | none |
| 2 | 30 000 € | 3 | 20 s | limit + 35 % | none |
| 3 | 70 000 € | 5 | 15 s | limit + 50 % | roadblocks |
| 4 | 150 000 € | 7 | 10 s | limit + 70 % | roadblocks, helicopter |
| 5 | 250 000 € | 10 | 5 s | limit + 90 % | roadblocks, helicopter, more ramming |

Police cars that crash out or fall 700 m behind are replaced after a while, to keep the
count. Without a helicopter, police more than 400 m away only know where you were 5 s
ago. With one, they always know where you are.

## Milestones

Each ends with its tests, clippy (native and wasm), `cargo fmt`, the notes and a commit.
The protocol is bumped in every milestone that changes a message.

1. **The event and the round, no police.** Kind `heist` in the events file, with
   `escape` lines under it (`events::parse`, the server's check that each point is near
   a road). `Game::Heist(heist::Settings { loot })`, `Play::Heist`,
   `Outcome::Heist(Escaped | Busted | Left)`, all on the wire. `Action::Heist(event)`
   opens a private round in a realm of its own. The client shows a window on the marker
   with the loot steps and its heat. Running: the escape point on the map and as a
   marker, the arrow and distance in the bar. Reaching it ends the round and pays the
   loot, for now back at the bank's marker. Tests: a heist runs in its own realm, and
   reaching the escape pays.
2. **The getaway.** The escort helicopter waits over the escape point (drawn for the
   round's player like the hunt's). On arrival the server takes the car, carries it to
   the garage park and pays. The client opens the hall when the car is set down there
   (a "put in the garage" message, or `Yours::moved` plus where). Test: the car ends at
   the garage park, paid.
3. **Police cars.** A police livery and lightbar (`Looks` gains a role, so remote cars
   draw it; flashing is local), and a pursuit driver in `traffic.rs` (a `Driver` kind or
   a `Pursuer` beside it): spawning out of sight 300–500 m away, routing towards the
   target (Dijkstra on the network, re-planned every second), going over the limit,
   overtaking traffic, aiming off the lane when near, and surviving small knocks. Only in
   a heist's realm. Tests: a police car reaches a car parked 1 km away along the roads,
   and one crashes out from a hard knock but not a light one.
4. **Busted and impounded.** The bust rule on the server and the meter in the bar.
   Impounding moves the car (and trunk) to the `impound` place in the database (a
   migration). Leaving mid-heist counts as busted. You land in the garage park's hall
   without a car: the new car-less state in the server (a peer with no car) and the game
   (only the hall, picking a garage car to drive out in). Test: sitting still with police
   near busts, the car is gone from `Owned`, the player has no car, then drives a garage
   car out.
4b. **"Get a free car".** In the hall with no car and an empty garage: the button, then a
   scene of its own (your parents' house: a drive, the start car with a big bow on it, a
   card "From Mum and Dad"), "Drive it" builds the server's start car and drives out. The
   server gives it only to a player with no car at all. Test: a car-less player with an
   empty garage gets the start car once, one with a garage car is refused.
5. **Heat.** The table above: counts, join delays, speeds. Roadblocks (two or three
   parked police cars across the road ahead on your likely way, placed out of sight).
   The police helicopter (a `Chase` per heist, sent like the hunt's) and what police know
   without it. Tests: at heat 1 a scripted drive to the escape gets away. At heat 5
   stopping halfway gets busted.
6. **The bank scene.** Like the other scenes, under the marker: the aftermath only (the
   user's answer, no robbers): a banking hall with counters, an open vault door, a red
   alarm light turning, money bags on a trolley and notes on the floor. The loot window moves into the scene, and the bags
   on the trolley grow with the amount. "Go!" drives you out into the heist's world.
   Shots for the notes and the screenshots skill.
7. **Polish.** Sirens (synthesised like the engine, two-tone, with Doppler through
   `hear_others`), police on the map, the round's chat line ("… got away with 70 000 €"),
   the cooldown, and the impound lot to buy a car back for a fine (a window-only shop,
   like the dealers).

Possible later: crew heists, losing the police (out of sight for long enough), spike
strips (tire pressures), loot bags as cargo that weigh the car down.

## Answers (2026-11-28)

- Losing the car is the stake. In the garage without a car: "Get a free car", a scene
  where your parents give you one (milestone 4b).
- Simple scripted police for now, to see how it feels.
- The bank scene shows the aftermath only.
- The first bank: Reckendorf, the Raiffeisen bank's building (under a made-up name).
