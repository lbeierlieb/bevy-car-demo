//! The catalogue on the network: the server sends it to every player on joining. Reading
//! checks that it holds together (every place it refers to is in it, every list that must
//! have something does), so a car built from it can't index out of it.

use super::{Catalogue, Clutch, Paint, Suspension, TireCompound};
use crate::body::{Body, Detail, Tone};
use crate::chassis::{Chassis, EngineBay, Gearbox, GearboxKind, Orientation, RollBars};
use crate::net::wire::{Reader, Wire};
use crate::physics::drivetrain::Drive;
use crate::physics::engine::families::{Family, Layout, Version};
use crate::physics::engine::hardware::{
    BottomEnd, CamProfile, Compressor, Cooling, Diverter, EngineHardware, Exhaust, Intake,
    Intercooler, Turbine, Turbo, ValveTrain, Wastegate,
};
use crate::physics::engine::parts::{
    Change, Effect, Field, Part, Preset, Readout, Regrind, Slot, TurboKit,
};
use crate::physics::fuel::Tank;
use crate::physics::tire::Compound;
use crate::wheels::WheelRoom;

/// The longest text in a catalogue (bytes).
const MAX_TEXT: usize = 2000;

/// Text longer than a chat line: names and descriptions.
struct Text(String);

impl Wire for Text {
    fn put(&self, out: &mut Vec<u8>) {
        let mut end = self.0.len().min(MAX_TEXT);
        while !self.0.is_char_boundary(end) {
            end -= 1;
        }
        (end as u16).put(out);
        out.extend_from_slice(&self.0.as_bytes()[..end]);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let n = usize::from(r.get::<u16>()?);
        (n <= MAX_TEXT).then_some(())?;
        Some(Self(String::from_utf8(r.take(n)?.to_vec()).ok()?))
    }
}

fn put_text(text: &str, out: &mut Vec<u8>) {
    Text(text.to_string()).put(out);
}

fn text(r: &mut Reader) -> Option<String> {
    Some(r.get::<Text>()?.0)
}

/// A struct as its fields one after another; `text` fields as `Text`.
macro_rules! fields {
    ($t:ty { $($f:ident),* $(,)? }) => {
        fields!($t { $($f),* } text {});
    };
    ($t:ty { $($f:ident),* $(,)? } text { $($s:ident),* $(,)? }) => {
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                $(put_text(&self.$s, out);)*
                $(self.$f.put(out);)*
            }
            fn get(r: &mut Reader) -> Option<Self> {
                $(let $s = text(r)?;)*
                Some(Self { $($s,)* $($f: r.get()?,)* })
            }
        }
    };
}

/// An enum without data, by its place in `all`.
macro_rules! listed {
    ($t:ty, $all:expr) => {
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                ($all.iter().position(|x| x == self).unwrap_or(0)).put(out);
            }
            fn get(r: &mut Reader) -> Option<Self> {
                $all.get(r.get::<usize>()?).copied()
            }
        }
    };
}

listed!(
    Tone,
    [
        Tone::Paint,
        Tone::Dark,
        Tone::Glass,
        Tone::Lamp,
        Tone::TailLamp,
        Tone::Metal
    ]
);
listed!(
    Orientation,
    [Orientation::Transverse, Orientation::Longitudinal]
);
listed!(
    GearboxKind,
    [
        GearboxKind::Transverse,
        GearboxKind::Inline,
        GearboxKind::Transaxle
    ]
);
listed!(Field, Field::ALL);
listed!(Readout, Readout::ALL);
listed!(Layout, Layout::ALL);

fields!(Detail { at, size, tone });
fields!(Tank { litres, at, height });
fields!(WheelRoom {
    diameter,
    width,
    min_rim
});
fields!(RollBars {
    sizes,
    factory,
    rate
});
fields!(EngineBay {
    orientation,
    length,
    width,
    height,
    engine_at,
    gearbox_at,
    header_room
} text { place });
fields!(Body {
    length,
    width,
    front_overhang,
    floor,
    deck,
    cabin,
    nose,
    tail,
    tumblehome,
    open_wheels,
    openings,
    details
});
fields!(Gearbox {
    kind,
    gears,
    reverse,
    mass,
    clutch_torque,
    shift_time,
    engage_time
} text { name });
fields!(Clutch { torque } text { name });
fields!(Suspension {
    ride,
    damping,
    camber,
    reach
} text { name });
fields!(Paint { color, shop } text { name });
fields!(Compound {
    grip,
    peak_angle,
    peak_ratio,
    c_lat,
    c_long,
    best,
    cold,
    overheat,
    life,
    rolling
});
fields!(TireCompound { rubber } text { name, about });
fields!(Chassis {
    mass,
    mass_at,
    cg_height,
    wheelbase,
    track,
    wheels,
    room,
    hub,
    ride,
    damping,
    bars,
    camber,
    pressure,
    max_steer,
    brakes,
    drag_area,
    downforce_area,
    tank,
    bay,
    drive,
    limited_slip,
    gearbox,
    final_drive,
    engines,
    paint,
    body,
    eye,
    insulation,
    traffic
} text { name, class, about });

impl Wire for Drive {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Drive::Rear => 0u8.put(out),
            Drive::Front => 1u8.put(out),
            Drive::All { front } => {
                2u8.put(out);
                front.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Drive::Rear,
            1 => Drive::Front,
            2 => Drive::All {
                front: r.get::<f32>()?.clamp(0.0, 1.0),
            },
            _ => return None,
        })
    }
}

fields!(CamProfile {
    intake_duration,
    exhaust_duration,
    lobe_separation,
    lift
});
fields!(BottomEnd {
    piston_mass,
    rod_mass,
    rod_strength,
    piston_toughness
});
fields!(ValveTrain {
    seat_force,
    rate,
    mass
});
fields!(Intake {
    filter_area,
    throttle_diameter,
    idle_valve_area,
    runner_length,
    plenum_volume,
    tract_volume
});
fields!(Compressor {
    diameter,
    head,
    choke,
    efficiency
});
fields!(Turbine {
    diameter,
    area,
    efficiency
});
fields!(Wastegate { area, spring, span });
fields!(Diverter { area, opens, span });
fields!(Intercooler {
    effectiveness,
    area
});
fields!(Turbo {
    compressor,
    turbine,
    inertia,
    wastegate,
    intercooler,
    diverter
});
fields!(Exhaust {
    header_length,
    pipe_area,
    cat_area,
    silencer_area
});
fields!(Cooling {
    radiator_area,
    radiator_effectiveness,
    thermostat_opens,
    thermostat_open,
    fan_on,
    fan_off,
    fan_speed,
    coolant_capacity,
    oil_capacity
});
fields!(EngineHardware {
    cylinders,
    bore,
    stroke,
    rod_length,
    compression_ratio,
    bottom_end,
    head_gasket,
    cams,
    intake_phasing,
    exhaust_phasing,
    intake_timing,
    exhaust_timing,
    valves,
    port_flow,
    intake,
    turbo,
    exhaust,
    cooling,
    inertia
} text { name });
fields!(Preset { chosen, boost, ecu });
fields!(Version { block, preset } text { code, name });
fields!(Family {
    layout,
    length,
    width,
    height,
    mass,
    versions
} text { name, about });

fields!(Regrind {
    duration,
    separation,
    lift
});
fields!(TurboKit {
    compressor,
    turbine,
    turbine_area,
    inertia,
    wastegate
});

impl Wire for Change {
    fn put(&self, out: &mut Vec<u8>) {
        let (kind, x) = match *self {
            Change::Times(x) => (0u8, x),
            Change::Over(x) => (1, x),
            Change::Plus(x) => (2, x),
            Change::To(x) => (3, x),
            Change::AtLeast(x) => (4, x),
        };
        kind.put(out);
        x.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let kind: u8 = r.get()?;
        let x: f32 = r.get()?;
        Some(match kind {
            0 => Change::Times(x),
            1 if x != 0.0 => Change::Over(x),
            2 => Change::Plus(x),
            3 => Change::To(x),
            4 => Change::AtLeast(x),
            _ => return None,
        })
    }
}

impl Wire for Effect {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Effect::Change {
                field,
                change,
                step,
            } => {
                0u8.put(out);
                field.put(out);
                change.put(out);
                step.put(out);
            }
            Effect::Remove(field) => {
                1u8.put(out);
                field.put(out);
            }
            Effect::Regrind(regrind) => {
                2u8.put(out);
                regrind.put(out);
            }
            Effect::Switching { mild, wild, wilder } => {
                3u8.put(out);
                mild.put(out);
                wild.put(out);
                wilder.put(out);
            }
            Effect::Turbo(kit) => {
                4u8.put(out);
                kit.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Effect::Change {
                field: r.get()?,
                change: r.get()?,
                step: r.get::<f32>()?.max(0.0),
            },
            1 => Effect::Remove(r.get()?),
            2 => Effect::Regrind(r.get()?),
            3 => Effect::Switching {
                mild: r.get()?,
                wild: r.get()?,
                wilder: r.get()?,
            },
            4 => Effect::Turbo(r.get()?),
            _ => return None,
        })
    }
}

fields!(Part { effects } text { name });
fields!(Slot {
    parts,
    readout,
    needs_turbo
} text { name, about });

impl Wire for Catalogue {
    fn put(&self, out: &mut Vec<u8>) {
        self.chassis.put(out);
        self.gearboxes.put(out);
        self.clutches.put(out);
        self.suspensions.put(out);
        self.paints.put(out);
        self.compounds.put(out);
        self.families.put(out);
        self.slots.put(out);
        self.start.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let catalogue = Catalogue::new(
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
            r.get()?,
        );
        catalogue.holds_together().then_some(catalogue)
    }
}

impl Catalogue {
    /// Whether every place it refers to is in it, and the lists cars need have something.
    pub fn holds_together(&self) -> bool {
        let wheels = |wheels: &[crate::wheels::Wheel; 2]| {
            (wheels.iter()).all(|wheel| wheel.compound < self.compounds.len())
        };
        let chassis = |chassis: &Chassis| {
            chassis.gearbox < self.gearboxes.len()
                && chassis.paint < self.paints.len()
                && wheels(&chassis.wheels)
                && (chassis.bars.iter()).all(|bars| bars.factory < bars.sizes.len())
                && !chassis.engines.is_empty()
                && (chassis.engines.iter()).all(|code| self.find_engine(code).is_some())
                && chassis.wheelbase > 0.5
                && chassis.mass > 50.0
        };
        let version = |version: &Version| {
            version.block.cylinders > 0
                && !version.block.cams.is_empty()
                && crate::physics::engine::parts::fits(self, &version.preset.chosen)
        };
        let slot = |slot: &Slot| !slot.parts.is_empty();
        let gearbox = |gearbox: &Gearbox| !gearbox.gears.is_empty();
        self.start < self.chassis.len()
            && !self.clutches.is_empty()
            && !self.suspensions.is_empty()
            && self.chassis.iter().all(chassis)
            && self.gearboxes.iter().all(gearbox)
            && self.slots.iter().all(slot)
            && (self.families.iter())
                .all(|f| !f.versions.is_empty() && f.versions.iter().all(version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;
    use crate::net::wire::{decode, encode};

    #[test]
    fn the_catalogue_reads_back_as_it_was() {
        let catalogue = builtin();
        assert!(catalogue.holds_together());
        let bytes = encode(&*catalogue);
        println!("the built-in catalogue is {} bytes", bytes.len());
        let read: Catalogue = decode(&bytes).expect("reads back");
        assert_eq!(read.fingerprint(), catalogue.fingerprint());
        assert_eq!(read.chassis, catalogue.chassis);
        assert_eq!(read.families, catalogue.families);
        assert_eq!(read.slots, catalogue.slots);
        assert_eq!(read.compounds, catalogue.compounds);
    }

    #[test]
    fn a_catalogue_that_refers_out_of_itself_is_refused() {
        let mut catalogue = Catalogue::clone(&builtin());
        catalogue.chassis[0].gearbox = catalogue.gearboxes.len();
        assert_eq!(decode::<Catalogue>(&encode(&catalogue)), None);
        let mut catalogue = Catalogue::clone(&builtin());
        catalogue.chassis[0].engines.push("Q99".into());
        assert_eq!(decode::<Catalogue>(&encode(&catalogue)), None);
    }
}
