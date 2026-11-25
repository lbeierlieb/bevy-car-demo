//! Where engine maps come from without baking: the maps of the built-in catalogue's
//! factory engines come baked with the game (`baked/`), found by their fingerprint. After
//! changing the cycle simulation, the maps' code or a factory engine, bake them again with
//! `cargo test --profile release-dev bake_stock_maps -- --ignored` (a test checks they're
//! current).

use super::hardware::EngineHardware;
use super::maps::{EngineMaps, fingerprint};

/// The maps baked with the game, by the factory code of their engine.
const BAKED: [(&str, &str); 22] = [
    ("A10", include_str!("baked/a10.maps")),
    ("A10T", include_str!("baked/a10t.maps")),
    ("B12", include_str!("baked/b12.maps")),
    ("B16", include_str!("baked/b16.maps")),
    ("B16S", include_str!("baked/b16s.maps")),
    ("D52", include_str!("baked/d52.maps")),
    ("F40", include_str!("baked/f40.maps")),
    ("H30", include_str!("baked/h30.maps")),
    ("H36T", include_str!("baked/h36t.maps")),
    ("L65", include_str!("baked/l65.maps")),
    ("M50", include_str!("baked/m50.maps")),
    ("M62", include_str!("baked/m62.maps")),
    ("R20", include_str!("baked/r20.maps")),
    ("R24", include_str!("baked/r24.maps")),
    ("S30", include_str!("baked/s30.maps")),
    ("S30T", include_str!("baked/s30t.maps")),
    ("V35", include_str!("baked/v35.maps")),
    ("V40", include_str!("baked/v40.maps")),
    ("W20", include_str!("baked/w20.maps")),
    ("W20T", include_str!("baked/w20t.maps")),
    ("X20T", include_str!("baked/x20t.maps")),
    ("X25T", include_str!("baked/x25t.maps")),
];

/// The text of the maps baked with the game for the hardware with `fingerprint`, if it's
/// a factory engine's: found by the fingerprint in the text's first lines.
fn stock_text(fingerprint: u64) -> Option<&'static str> {
    let tag = format!("fingerprint {fingerprint:016x}");
    (BAKED.iter())
        .find(|(_, text)| text.lines().take(3).any(|line| line == tag))
        .map(|(_, text)| *text)
}

/// Whether the maps for `hardware` come baked with the game.
pub fn is_stock(hardware: &EngineHardware) -> bool {
    stock_text(fingerprint(hardware)).is_some()
}

impl EngineMaps {
    /// The maps for `hardware`: baked with the game for the stock builds, or baked now.
    pub fn for_hardware(hardware: &EngineHardware) -> Self {
        Self::stock(hardware).unwrap_or_else(|| Self::bake(hardware))
    }

    /// The maps baked with the game, if `hardware` is (on the cylinder side) a factory
    /// engine.
    pub fn stock(hardware: &EngineHardware) -> Option<Self> {
        let fingerprint = fingerprint(hardware);
        Self::from_text(stock_text(fingerprint)?, fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;
    use crate::physics::engine::parts::Parts;

    /// The built-in catalogue's factory engines: the hardware, the file its maps are in,
    /// and the maps' text.
    fn stock() -> Vec<(EngineHardware, String, &'static str)> {
        let catalogue = builtin();
        let mut stock = Vec::new();
        for (f, family) in catalogue.families.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                let hardware = Parts::of_version(&catalogue, f, v).hardware();
                let file = format!("{}.maps", version.code.to_lowercase());
                let text = (BAKED.iter())
                    .find(|(code, _)| *code == version.code)
                    .map_or("", |(_, text)| *text);
                stock.push((hardware, file, text));
            }
        }
        stock
    }

    #[test]
    #[ignore = "bakes the stock maps into src/physics/engine/baked"]
    fn bake_stock_maps() {
        for (hardware, file, _) in stock() {
            let path = format!(
                "{}/src/physics/engine/baked/{file}",
                env!("CARGO_MANIFEST_DIR")
            );
            let text = EngineMaps::bake(&hardware).to_text(&hardware);
            std::fs::write(&path, text).unwrap();
        }
    }

    #[test]
    fn stock_maps_are_current() {
        for (hardware, file, text) in stock() {
            let maps = EngineMaps::from_text(text, fingerprint(&hardware));
            assert!(
                maps.is_some(),
                "{file} is out of date: cargo test --profile release-dev bake_stock_maps -- --ignored"
            );
            assert_eq!(maps.unwrap().to_text(&hardware), text);
        }
    }
}
