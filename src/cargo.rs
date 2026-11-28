//! What a car carries: loose parts in its hold. Each chassis has a hold (`Hold`): a grid of
//! 10 cm cells over the floor behind the front seats, in areas with a floor and a ceiling
//! each (the trunk under its cover, the rear bench's place, the passenger seat's), the
//! seats' areas only there while the seat is out. The game packs what's loaded (nobody
//! places parts by hand): the largest first, each turned whichever way and put where its
//! top ends lowest, resting on the floor or on what's under it. Packed again from scratch
//! whenever what's carried changes, it comes out the same on the server and in the game, so
//! where each part lies (and so its mass) needs no saying.

use bevy::prelude::*;

use crate::catalogue::Catalogue;
use crate::items::{Does, Item, Kind};

/// A cell's side (m).
pub const CELL: f32 = 0.1;

/// The most parts a car carries (against endless lists: a hold fills up long before).
pub const MOST: usize = 64;

/// A chassis' hold: where its cells are and its areas.
#[derive(Clone, Debug, PartialEq)]
pub struct Hold {
    /// Where cell (0, 0, 0)'s corner is: metres from the nose (its front), the body's x
    /// (its left side; the driver sits on the left, at negative x) and above the ground.
    pub from: f32,
    pub left: f32,
    pub base: f32,
    pub areas: Vec<Area>,
}

/// A part of the hold: cells `along` (from the front) and `across` (from the left), from
/// `floor` up to `ceiling` (cells); only while the seat slot named `seat` is empty, if
/// it's a seat's place.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub along: [u8; 2],
    pub across: [u8; 2],
    pub floor: u8,
    pub ceiling: u8,
    pub seat: Option<String>,
}

/// Where a part lies: its corner cell (along, across, up) and how many cells it takes each
/// way (as it's turned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stowed {
    pub at: [u8; 3],
    pub size: [u8; 3],
}

/// The hold's room as it is: each column's floor and ceiling (cells; no room where
/// they're equal).
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub along: usize,
    pub across: usize,
    columns: Vec<(u8, u8)>,
}

impl Grid {
    /// `hold`'s room, the seats' areas there if `open` says their slot is empty.
    pub fn of(hold: &Hold, open: impl Fn(&str) -> bool) -> Self {
        let along = (hold.areas.iter()).map(|a| a.along[1]).max().unwrap_or(0) as usize;
        let across = (hold.areas.iter()).map(|a| a.across[1]).max().unwrap_or(0) as usize;
        let mut columns = vec![(0, 0); along * across];
        for area in &hold.areas {
            if area.seat.as_deref().is_some_and(|seat| !open(seat)) {
                continue;
            }
            for i in area.along[0]..area.along[1] {
                for j in area.across[0]..area.across[1] {
                    columns[usize::from(i) * across + usize::from(j)] = (area.floor, area.ceiling);
                }
            }
        }
        Self {
            along,
            across,
            columns,
        }
    }

    /// No room in the cells whose middle is `along` (metres from the nose; the hold's
    /// cells start at `from`).
    pub fn close(&mut self, from: f32, along: std::ops::Range<f32>) {
        for i in 0..self.along {
            let middle = from + (i as f32 + 0.5) * CELL;
            if along.contains(&middle) {
                for j in 0..self.across {
                    self.columns[i * self.across + j] = (0, 0);
                }
            }
        }
    }

    /// How many cells there's room for.
    pub fn room(&self) -> usize {
        (self.columns.iter())
            .map(|&(floor, ceiling)| usize::from(ceiling.saturating_sub(floor)))
            .sum()
    }

    /// Packs boxes of `sizes` (m: length, width, height): where each lies, in their order,
    /// or the first (in their order) that found no room.
    pub fn pack(&self, sizes: &[[f32; 3]]) -> Result<Vec<Stowed>, usize> {
        let cells: Vec<[u8; 3]> = sizes.iter().map(|size| size.map(cells)).collect();
        // The largest first (by volume, then the longest side), else as they come.
        let mut order: Vec<usize> = (0..cells.len()).collect();
        let volume = |c: &[u8; 3]| c.iter().map(|&n| u32::from(n)).product::<u32>();
        order.sort_by_key(|&i| {
            let c = &cells[i];
            (
                std::cmp::Reverse(volume(c)),
                std::cmp::Reverse(*c.iter().max().unwrap_or(&0)),
                i,
            )
        });
        // What's packed so far: each column's top.
        let mut tops: Vec<u8> = self.columns.iter().map(|&(floor, _)| floor).collect();
        let mut stowed = vec![None; cells.len()];
        let mut failed = None;
        for i in order {
            match self.place(&tops, cells[i]) {
                Some(spot) => {
                    let [a, b, h] = spot.size;
                    for di in 0..usize::from(a) {
                        for dj in 0..usize::from(b) {
                            let column = (usize::from(spot.at[0]) + di) * self.across
                                + usize::from(spot.at[1])
                                + dj;
                            tops[column] = spot.at[2] + h;
                        }
                    }
                    stowed[i] = Some(spot);
                }
                None => {
                    failed = Some(failed.map_or(i, |f: usize| f.min(i)));
                }
            }
        }
        match failed {
            Some(i) => Err(i),
            None => Ok(stowed.into_iter().map(|s| s.expect("stowed")).collect()),
        }
    }

    /// Where a box of `cells` goes on `tops`: turned every way, the place where its top ends
    /// lowest, then the frontmost, then the leftmost.
    fn place(&self, tops: &[u8], cells: [u8; 3]) -> Option<Stowed> {
        let [x, y, z] = cells;
        let mut turns = vec![
            [x, y, z],
            [y, x, z],
            [x, z, y],
            [z, x, y],
            [y, z, x],
            [z, y, x],
        ];
        // Lying flat first (where they'd end as high, the flatter one wins).
        turns.sort_by_key(|t| t[2]);
        turns.dedup();
        let mut best: Option<((u8, usize, usize), Stowed)> = None;
        for size in turns {
            let [a, b, h] = size.map(usize::from);
            if a > self.along || b > self.across {
                continue;
            }
            for i in 0..=self.along - a {
                for j in 0..=self.across - b {
                    let mut bottom = 0u8;
                    let mut ceiling = u8::MAX;
                    for di in 0..a {
                        for dj in 0..b {
                            let column = (i + di) * self.across + j + dj;
                            let (floor, top) = self.columns[column];
                            if floor == top {
                                ceiling = 0;
                            }
                            bottom = bottom.max(tops[column]);
                            ceiling = ceiling.min(top);
                        }
                    }
                    let end = usize::from(bottom) + h;
                    if end > usize::from(ceiling) {
                        continue;
                    }
                    let score = (end as u8, i, j);
                    if best.as_ref().is_none_or(|(b, _)| score < *b) {
                        let spot = Stowed {
                            at: [i as u8, j as u8, bottom],
                            size,
                        };
                        best = Some((score, spot));
                    }
                }
            }
        }
        best.map(|(_, spot)| spot)
    }
}

/// How many cells `metres` take (a hair over a whole cell counts as the cell).
fn cells(metres: f32) -> u8 {
    let n = (metres / CELL - 0.01).ceil();
    if n.is_finite() {
        n.clamp(1.0, 255.0) as u8
    } else {
        255
    }
}

/// The hold of the chassis `car` (a car's tree) is, as it is with the seats `car` has
/// fitted.
pub fn grid(catalogue: &Catalogue, car: &Item) -> Option<Grid> {
    let Does::Chassis(chassis) = car.def(catalogue).does else {
        return None;
    };
    let def = car.def(catalogue);
    let open = |seat: &str| {
        (def.slots.iter().zip(&car.slots)).any(|(slot, item)| {
            slot.name == seat && matches!(slot.kind, Kind::Seat(_)) && item.is_none()
        })
    };
    let made = &catalogue.chassis[chassis];
    let mut grid = Grid::of(&made.hold, open);
    // An engine subframe's bay takes the hold's room along its length.
    if let Some((_, _, bay)) = crate::items::engine_subframe(catalogue, car) {
        let middle = made.body.front_overhang + bay.engine_at;
        let (from, to) = (middle - bay.length / 2.0, middle + bay.length / 2.0);
        grid.close(made.hold.from, from..to);
    }
    Some(grid)
}

/// An item's mass with everything fitted to it (kg).
pub fn mass(catalogue: &Catalogue, item: &Item) -> f32 {
    item.all().iter().map(|item| item.def(catalogue).mass).sum()
}

/// Where `cargo` lies in `car`'s hold, or the first that has no room there.
pub fn stow(catalogue: &Catalogue, car: &Item, cargo: &[Item]) -> Result<Vec<Stowed>, usize> {
    if cargo.len() > MOST {
        return Err(MOST);
    }
    let Some(grid) = grid(catalogue, car) else {
        return Err(0);
    };
    let sizes: Vec<[f32; 3]> = cargo.iter().map(|item| item.def(catalogue).size).collect();
    grid.pack(&sizes)
}

/// As many of `cargo` as `car` has room for, in order (one with no room is left out).
pub fn fitting(catalogue: &Catalogue, car: &Item, mut cargo: Vec<Item>) -> Vec<Item> {
    cargo.truncate(MOST);
    while let Err(i) = stow(catalogue, car, &cargo) {
        if i >= cargo.len() {
            return Vec::new();
        }
        cargo.remove(i);
    }
    cargo
}

/// Whether `car` can carry `cargo` and `more` besides.
pub fn fits(catalogue: &Catalogue, car: &Item, cargo: &[Item], more: &Item) -> bool {
    let mut all = cargo.to_vec();
    all.push(more.clone());
    stow(catalogue, car, &all).is_ok()
}

/// Each part of `cargo` (stowed as `stowed`) as a mass at its middle in the body's design
/// frame (x right, y up from the ground, z backwards from midway between the axles).
pub fn masses(
    catalogue: &Catalogue,
    car: &Item,
    cargo: &[Item],
    stowed: &[Stowed],
) -> Vec<(f32, Vec3)> {
    let Does::Chassis(chassis) = car.def(catalogue).does else {
        return Vec::new();
    };
    let chassis = &catalogue.chassis[chassis];
    let hold = &chassis.hold;
    let to_middle = chassis.body.front_overhang + chassis.wheelbase / 2.0;
    (cargo.iter().zip(stowed))
        .map(|(item, spot)| {
            let middle = |n: usize| (f32::from(spot.at[n]) + 0.5 * f32::from(spot.size[n])) * CELL;
            let at = Vec3::new(
                hold.left + middle(1),
                hold.base + middle(2),
                hold.from + middle(0) - to_middle,
            );
            (mass(catalogue, item), at)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;

    fn def(catalogue: &Catalogue, id: &str) -> Item {
        let def = catalogue.defs.iter().position(|d| d.id == id).expect(id);
        Item::new(catalogue, def, 0)
    }

    /// The hatch's seats out.
    fn without(catalogue: &Catalogue, car: &Item, seats: &[&str]) -> Item {
        let mut car = car.clone();
        let slots = car.def(catalogue).slots.clone();
        for (slot, item) in slots.iter().zip(&mut car.slots) {
            if seats.contains(&slot.name.as_str()) {
                assert!(matches!(slot.kind, Kind::Seat(_)));
                *item = None;
            }
        }
        car
    }

    #[test]
    fn boxes_rest_on_the_floor_and_on_each_other() {
        let hold = Hold {
            from: 0.0,
            left: 0.0,
            base: 0.0,
            areas: vec![Area {
                along: [0, 4],
                across: [0, 4],
                floor: 0,
                ceiling: 4,
                seat: None,
            }],
        };
        let grid = Grid::of(&hold, |_| true);
        assert_eq!(grid.room(), 64);
        // A flat slab lies flat; a cube goes on top of the slab, or next to it if there's
        // room there.
        let packed = grid.pack(&[[0.2, 0.2, 0.2], [0.4, 0.4, 0.1]]).unwrap();
        assert_eq!(
            packed[1],
            Stowed {
                at: [0, 0, 0],
                size: [4, 4, 1]
            }
        );
        assert_eq!(packed[0].at[2], 1);
        // Too much is the first that didn't fit.
        let bricks = vec![[0.4, 0.4, 0.2]; 3];
        assert_eq!(grid.pack(&bricks), Err(2));
        // Turned to fit: a long thing stands up.
        assert!(grid.pack(&[[0.1, 0.1, 0.4]]).is_ok());
        assert!(grid.pack(&[[0.5, 0.1, 0.1]]).is_err());
    }

    #[test]
    fn the_hatch_carries_more_with_its_seats_out() {
        let catalogue = builtin();
        let car = &catalogue.trims[0].car;
        let wheel = car.fitted(&catalogue, Kind::Wheel).next().unwrap().clone();
        // A wheel lies in the trunk; a second has no room.
        assert!(fits(&catalogue, car, &[], &wheel));
        assert!(!fits(&catalogue, car, std::slice::from_ref(&wheel), &wheel));
        // Without the rear bench, all four and a silencer.
        let open = without(&catalogue, car, &["Rear bench"]);
        let four = vec![wheel.clone(); 4];
        assert!(stow(&catalogue, &open, &four).is_ok());
        // The exhaust pipe goes through from the passenger's footwell to the hatch.
        let pipe = def(&catalogue, "exhaust-pipe.stock");
        assert!(!fits(&catalogue, &open, &[], &pipe));
        let empty = without(&catalogue, car, &["Rear bench", "Passenger seat"]);
        assert!(fits(&catalogue, &empty, &[], &pipe));
    }

    #[test]
    fn what_is_carried_weighs_where_it_lies() {
        let catalogue = builtin();
        let car = &catalogue.trims[0].car;
        let silencer = def(&catalogue, "silencer.stock");
        let cargo = vec![silencer.clone()];
        let stowed = stow(&catalogue, car, &cargo).expect("a silencer fits the trunk");
        let masses = masses(&catalogue, car, &cargo, &stowed);
        assert_eq!(masses[0].0, 7.0);
        // Behind the rear axle (z > half the wheelbase), above the floor.
        let half = catalogue.chassis[0].wheelbase / 2.0;
        assert!(masses[0].1.z > half && masses[0].1.y > 0.4, "{masses:?}");
        // The same packing whatever order they're loaded in.
        let wheel = car.fitted(&catalogue, Kind::Wheel).next().unwrap().clone();
        let open = without(&catalogue, car, &["Rear bench"]);
        let seat = def(&catalogue, "seat.stock");
        let a = vec![seat.clone(), wheel.clone()];
        let b = vec![wheel, seat];
        let (sa, sb) = (
            stow(&catalogue, &open, &a).unwrap(),
            stow(&catalogue, &open, &b).unwrap(),
        );
        assert_eq!((sa[0], sa[1]), (sb[1], sb[0]));
    }
}
