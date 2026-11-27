//! What a player owns besides the car they drive: cars and parts in their garage (one
//! place on the map, the garage park, everyone their own garage there), and parts held for
//! them at shops (left there, or shipped there from the garage). The server keeps it (in
//! its database, between sessions, for players with an account) and does what players ask
//! (`Stash`): load a part into the trunk, unload one, ship one, drive another car. Each
//! player is told what they own (`OwnedView`) whenever it changes.

use crate::catalogue::Catalogue;
use crate::items::{Item, Kind};
use crate::net::wire::{Reader, Wire};
use crate::saved::Condition;
use crate::spec::CarSpec;

/// The most cars and parts a garage holds, and parts a shop holds for a player: high
/// enough never to be hit by playing, only against endless growth.
pub const GARAGE_CARS: usize = 50;
pub const GARAGE_PARTS: usize = 2000;
pub const SHOP_HOLD: usize = 20;

/// A car in the garage: what it's built of (and carries) and how worn it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Parked {
    pub spec: CarSpec,
    pub condition: Condition,
}

/// What a player owns besides the car they drive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Owned {
    /// In the garage.
    pub cars: Vec<Parked>,
    pub parts: Vec<Item>,
    /// Held at shops, by the shop's name.
    pub held: Vec<(String, Vec<Item>)>,
    /// Counts the changes (for saving only what changed).
    pub revision: u32,
}

impl Owned {
    /// Something changed.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// The parts held at the shop called `shop`.
    pub fn held_at(&self, shop: &str) -> &[Item] {
        (self.held.iter())
            .find(|(name, _)| name == shop)
            .map_or(&[], |(_, items)| items.as_slice())
    }

    pub fn held_at_mut(&mut self, shop: &str) -> &mut Vec<Item> {
        let place = match self.held.iter().position(|(name, _)| name == shop) {
            Some(place) => place,
            None => {
                self.held.push((shop.to_string(), Vec::new()));
                self.held.len() - 1
            }
        };
        &mut self.held[place].1
    }

    /// What the player is told of it, the shops by their places in `shops` (the server's
    /// list; parts held at a shop no longer in it aren't shown).
    pub fn view(&self, shops: &[crate::shops::Shop]) -> OwnedView {
        OwnedView {
            money: 0,
            cars: (self.cars.iter())
                .map(|parked| ParkedView {
                    car: parked.spec.car.clone(),
                    cargo: parked.spec.cargo.clone(),
                })
                .collect(),
            parts: self.parts.clone(),
            held: (shops.iter().enumerate())
                .filter_map(|(i, shop)| {
                    let items = self.held_at(&shop.name);
                    (!items.is_empty()).then(|| (i as u16, items.to_vec()))
                })
                .collect(),
        }
    }
}

/// A car in the garage as its owner sees it: what it's built of and carries.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkedView {
    pub car: Item,
    pub cargo: Vec<Item>,
}

/// What a player owns as they're told it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnedView {
    /// What's in their wallet (cents).
    pub money: i64,
    pub cars: Vec<ParkedView>,
    pub parts: Vec<Item>,
    /// Parts held at shops (by their places in the server's list of shops).
    pub held: Vec<(u16, Vec<Item>)>,
}

impl OwnedView {
    /// The parts held at shop `shop` (its place in the list).
    pub fn held_at(&self, shop: usize) -> &[Item] {
        (self.held.iter())
            .find(|(i, _)| usize::from(*i) == shop)
            .map_or(&[], |(_, items)| items.as_slice())
    }
}

/// Where a part is that a player does something with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// In the trunk of the car they drive.
    Trunk,
    /// Here: in the garage at the garage park, else held at the shop they're at.
    Here,
}

/// What a player asks of what they own. Parts go by their place in the list they're in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stash {
    /// From the trunk into the garage, or left at the shop (held for them there).
    Unload { item: u16 },
    /// Into the trunk, from the garage or from what the shop holds for them.
    Load { item: u16 },
    /// Shipped to the garage (at a shop): from the trunk or what the shop holds.
    ShipHome { from: Source, item: u16 },
    /// At the garage park: a part from the garage shipped to a shop (its place in the
    /// list), which holds it for them.
    Ship { item: u16, shop: u16 },
    /// At the garage park: drive the garage's car `car` (the one driven parked in the
    /// garage in its place).
    Drive { car: u16 },
}

impl Wire for Source {
    fn put(&self, out: &mut Vec<u8>) {
        (*self == Source::Here).put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(if r.get::<bool>()? {
            Source::Here
        } else {
            Source::Trunk
        })
    }
}

impl Wire for Stash {
    fn put(&self, out: &mut Vec<u8>) {
        match *self {
            Stash::Unload { item } => (0u8, item).put(out),
            Stash::Load { item } => (1u8, item).put(out),
            Stash::ShipHome { from, item } => {
                2u8.put(out);
                from.put(out);
                item.put(out);
            }
            Stash::Ship { item, shop } => {
                3u8.put(out);
                item.put(out);
                shop.put(out);
            }
            Stash::Drive { car } => (4u8, car).put(out),
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Stash::Unload { item: r.get()? },
            1 => Stash::Load { item: r.get()? },
            2 => Stash::ShipHome {
                from: r.get()?,
                item: r.get()?,
            },
            3 => Stash::Ship {
                item: r.get()?,
                shop: r.get()?,
            },
            4 => Stash::Drive { car: r.get()? },
            _ => return None,
        })
    }
}

/// Loose parts (never a car).
fn loose(r: &mut Reader) -> Option<Vec<Item>> {
    let items: Vec<Item> = r.get()?;
    let catalogue = r.catalogue()?;
    (items.iter())
        .all(|item| item.def(catalogue).kind != Kind::Chassis)
        .then_some(items)
}

impl Wire for ParkedView {
    fn put(&self, out: &mut Vec<u8>) {
        self.car.put(out);
        self.cargo.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let car: Item = r.get()?;
        let catalogue: &Catalogue = r.catalogue()?;
        (car.def(catalogue).kind == Kind::Chassis).then_some(())?;
        Some(Self {
            car,
            cargo: loose(r)?,
        })
    }
}

impl Wire for OwnedView {
    fn put(&self, out: &mut Vec<u8>) {
        self.money.put(out);
        self.cars.put(out);
        self.parts.put(out);
        (self.held.len() as u16).put(out);
        for (shop, items) in &self.held {
            shop.put(out);
            items.put(out);
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let money = r.get()?;
        let cars = r.get()?;
        let parts = loose(r)?;
        let n: u16 = r.get()?;
        let held = (0..n)
            .map(|_| Some((r.get()?, loose(r)?)))
            .collect::<Option<_>>()?;
        Some(Self {
            money,
            cars,
            parts,
            held,
        })
    }
}

/// What a part is called, for people.
pub fn name(catalogue: &Catalogue, item: &Item) -> String {
    let def = item.def(catalogue);
    match def.does {
        crate::items::Does::Rim => crate::wheels::Rim::all()
            .nth(item.variant)
            .map_or(def.name.clone(), |rim| {
                format!("{} {}", def.name, rim.label())
            }),
        crate::items::Does::Tire(_) => crate::wheels::TireSize::all()
            .nth(item.variant)
            .map_or(def.name.clone(), |size| {
                format!("{} {}", def.name, size.label())
            }),
        crate::items::Does::Engine => item
            .all()
            .into_iter()
            .find_map(|part| match part.def(catalogue).does {
                crate::items::Does::Block { family, version } => {
                    let version = &catalogue.families[family].versions[version];
                    Some(format!("{} {} engine", version.code, version.name))
                }
                _ => None,
            })
            .unwrap_or_else(|| format!("{} (no long block)", def.name)),
        crate::items::Does::Wheel => {
            let inner: Vec<String> = item
                .slots
                .iter()
                .flatten()
                .map(|i| name(catalogue, i))
                .collect();
            if inner.is_empty() {
                def.name.clone()
            } else {
                format!("Wheel: {}", inner.join(", "))
            }
        }
        _ => def.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;
    use crate::net::wire::{decode_with, encode};

    #[test]
    fn what_a_player_owns_goes_over_the_wire() {
        let catalogue = builtin();
        let car = &catalogue.trims[0].car;
        let wheel = car.fitted(&catalogue, Kind::Wheel).next().unwrap().clone();
        let view = OwnedView {
            money: 123_456,
            cars: vec![ParkedView {
                car: car.clone(),
                cargo: vec![wheel.clone()],
            }],
            parts: vec![wheel.clone(), wheel.clone()],
            held: vec![(3, vec![wheel])],
        };
        let read: OwnedView = decode_with(&encode(&view), Some(&catalogue)).unwrap();
        assert_eq!(read, view);
        // A car isn't a loose part.
        let wrong = OwnedView {
            parts: vec![car.clone()],
            ..view
        };
        assert!(decode_with::<OwnedView>(&encode(&wrong), Some(&catalogue)).is_none());
        for stash in [
            Stash::Unload { item: 2 },
            Stash::ShipHome {
                from: Source::Here,
                item: 1,
            },
            Stash::Ship { item: 4, shop: 7 },
            Stash::Drive { car: 1 },
        ] {
            assert_eq!(decode_with(&encode(&stash), None), Some(stash));
        }
    }
}
