//! A car's state between simulation steps as bytes: everything that changes as it drives
//! (the body's motion, the wheels, tires, engine, gearbox, fuel), not what its setup
//! decides. Loaded into a car built from the same setup, it carries on exactly as the
//! saved one would (`a_loaded_car_drives_on_as_the_saved_one`), so the server can hand a
//! player its car's state and the player's game can take it up. Values are written as
//! they are, to the bit, and checked when read (finite floats, sane rotations, catalogue
//! indices).

use bevy::prelude::*;

use crate::net::wire::{Reader, Wire};

/// State that can be saved and loaded into an existing value (one built from the same
/// setup, which keeps what the setup decides).
pub trait Snapshot {
    fn save(&self, out: &mut Vec<u8>);
    /// Nothing, if the bytes are malformed (the value may then be half loaded).
    fn load(&mut self, r: &mut Reader) -> Option<()>;
}

/// Snapshots of plain values, by their wire encoding.
macro_rules! leaf {
    ($($t:ty),*) => {$(
        impl Snapshot for $t {
            fn save(&self, out: &mut Vec<u8>) {
                self.put(out);
            }
            fn load(&mut self, r: &mut Reader) -> Option<()> {
                *self = r.get()?;
                Some(())
            }
        }
    )*};
}
leaf!(u8, i8, u32, f32, f64, bool, Vec2, Vec3);

/// A catalogue index or count (one byte).
impl Snapshot for usize {
    fn save(&self, out: &mut Vec<u8>) {
        self.put(out);
    }
    fn load(&mut self, r: &mut Reader) -> Option<()> {
        *self = r.get()?;
        Some(())
    }
}

/// A rotation exactly as it is (the wire's `Quat` normalises it).
impl Snapshot for Quat {
    fn save(&self, out: &mut Vec<u8>) {
        self.to_array().put(out);
    }
    fn load(&mut self, r: &mut Reader) -> Option<()> {
        let q = Quat::from_array(r.get()?);
        (0.5..2.0).contains(&q.length_squared()).then(|| *self = q)
    }
}

impl<T: Snapshot, const N: usize> Snapshot for [T; N] {
    fn save(&self, out: &mut Vec<u8>) {
        for x in self {
            x.save(out);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Option<()> {
        for x in self {
            x.load(r)?;
        }
        Some(())
    }
}

impl<T: Snapshot + Default> Snapshot for Option<T> {
    fn save(&self, out: &mut Vec<u8>) {
        self.is_some().put(out);
        if let Some(x) = self {
            x.save(out);
        }
    }
    fn load(&mut self, r: &mut Reader) -> Option<()> {
        *self = if r.get::<bool>()? {
            let mut x = T::default();
            x.load(r)?;
            Some(x)
        } else {
            None
        };
        Some(())
    }
}

impl<A: Snapshot, B: Snapshot> Snapshot for (A, B) {
    fn save(&self, out: &mut Vec<u8>) {
        self.0.save(out);
        self.1.save(out);
    }
    fn load(&mut self, r: &mut Reader) -> Option<()> {
        self.0.load(r)?;
        self.1.load(r)
    }
}

/// A snapshot of a struct made of the fields listed (the others are its setup's): used in
/// the struct's own module, so private fields can be listed.
macro_rules! snapshot {
    ($t:ty { $($field:ident),* $(,)? }) => {
        impl $crate::snapshot::Snapshot for $t {
            fn save(&self, out: &mut Vec<u8>) {
                $($crate::snapshot::Snapshot::save(&self.$field, out);)*
            }
            fn load(&mut self, r: &mut $crate::net::wire::Reader) -> Option<()> {
                $($crate::snapshot::Snapshot::load(&mut self.$field, r)?;)*
                Some(())
            }
        }
    };
}
pub(crate) use snapshot;

/// Saves `value` into new bytes.
pub fn save<T: Snapshot>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.save(&mut out);
    out
}

/// Loads all of `bytes` into `value`; false (and `value` possibly half loaded) if they're
/// malformed or there are bytes left over.
pub fn load<T: Snapshot>(value: &mut T, bytes: &[u8]) -> bool {
    let mut r = Reader::new(bytes);
    value.load(&mut r).is_some() && r.done()
}
