# Heists (planned 2026-11-28, waiting for the user's answers on the assumptions)

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

## Assumptions (to confirm)

Each comes with what I'd do. The ones most worth an answer come first.

1. **Solo.** A heist is a private round of one, like a time trial. Crew heists (a
   getaway driver plus others) come later, if wanted.
2. **Your own car is the stake.** No lent cars. Being busted takes the car you drove,
   with its trunk.
3. **Impound** means the car goes to a new place, `impound`, in the database. It isn't
   destroyed. You get it back at an impound lot (a new shop kind) for a fine, built in
   the last milestone. Until then an impounded car is gone for good.
4. **After a bust** you are put in the garage park's hall. The server makes your first
   garage car the one you drive. With an empty garage, it gives you the server's start
   car, as a new player gets, so nobody ends up with no car. Being without a car would
   touch every system, so I'd avoid it.
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
11. **Police are kinematic, like traffic, not simulated cars.** A simulated car costs
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
15. **The bank** is a made-up name ("Kreditkasse Baunach" or similar) at a real building
    in Baunach, its point picked from the map in milestone 1, with three escape points
    1–4 km away.

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
   migration). Then the next garage car or the start car, the move to the garage park,
   and the hall. Leaving mid-heist counts as busted. Test: sitting still with police
   near busts, the car is gone from `Owned`, you drive another at the garage park.
5. **Heat.** The table above: counts, join delays, speeds. Roadblocks (two or three
   parked police cars across the road ahead on your likely way, placed out of sight).
   The police helicopter (a `Chase` per heist, sent like the hunt's) and what police know
   without it. Tests: at heat 1 a scripted drive to the escape gets away. At heat 5
   stopping halfway gets busted.
6. **The bank scene.** Like the other scenes, under the marker: a banking hall with
   counters, an open vault door, a red alarm light turning, money bags on a trolley and
   notes on the floor. There are no people, as the game has none. Masked robbers made of
   primitives are possible if wanted. The loot window moves into the scene, and the bags
   on the trolley grow with the amount. "Go!" drives you out into the heist's world.
   Shots for the notes and the screenshots skill.
7. **Polish.** Sirens (synthesised like the engine, two-tone, with Doppler through
   `hear_others`), police on the map, the round's chat line ("… got away with 70 000 €"),
   the cooldown, and the impound lot to buy a car back for a fine (a window-only shop,
   like the dealers).

Possible later: crew heists, losing the police (out of sight for long enough), spike
strips (tire pressures), loot bags as cargo that weigh the car down.

## Open questions for the user

- Assumptions 2 to 4: is losing the car for good (until the impound lot comes) the
  stake you want, and what happens with an empty garage?
- Assumption 11: kinematic police (cheap, many, a bit rail-like) or simulated cars
  (real physics, few, costly on the VPS)?
- Should the bank scene have robbers in it, or just the aftermath (vault open, alarm)?
- Which town and building for the first bank?
