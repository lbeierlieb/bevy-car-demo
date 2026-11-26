//! Where engine maps come from without baking: the maps of the built-in catalogue's
//! factory engines come baked with the game (`baked/`), found by their fingerprint; not on
//! the web, which only plays on a server and gets them from it. After
//! changing the cycle simulation, the maps' code or a factory engine, bake them again with
//! `cargo test --profile release-dev bake_stock_maps -- --ignored` (a test checks they're
//! current).

use super::hardware::EngineHardware;
use super::maps::{EngineMaps, fingerprint};

/// The maps baked with the game, by the factory code of their engine; and two engines
/// the tests measure against (`REFERENCES`).
#[cfg(any(test, not(target_arch = "wasm32")))]
const BAKED: [(&str, &str); 4] = [
    ("B16", include_str!("baked/b16.maps")),
    ("B18", include_str!("baked/b18.maps")),
    ("W20", include_str!("baked/w20.maps")),
    ("B16T", include_str!("baked/b16t.maps")),
];

/// None on the web, which has no maps baked with it.
#[cfg(all(not(test), target_arch = "wasm32"))]
fn stock_text(_: u64) -> Option<&'static str> {
    None
}

/// The text of the maps baked with the game for the hardware with `fingerprint`, if it's
/// a factory engine's: found by the fingerprint in the text's first lines.
#[cfg(any(test, not(target_arch = "wasm32")))]
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

/// The maps baked with the game for the hardware with `fingerprint`, as compressed text
/// for the network (see `unpack`).
pub fn packed(fingerprint: u64) -> Option<Vec<u8>> {
    let text = stock_text(fingerprint)?;
    Some(miniz_oxide::deflate::compress_to_vec_zlib(
        text.as_bytes(),
        6,
    ))
}

/// Maps' text from `packed`.
pub fn unpack(bytes: &[u8]) -> Option<String> {
    let text = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(bytes, 16 << 20).ok()?;
    String::from_utf8(text).ok()
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
        // The engines tests measure against: the 2-litre four everything is scaled from,
        // and the 1.6 turbocharged.
        let references = [
            ("W20", EngineHardware::default()),
            ("B16T", EngineHardware::turbo()),
        ];
        for (code, hardware) in references {
            let file = format!("{}.maps", code.to_lowercase());
            let text = (BAKED.iter())
                .find(|(baked, _)| *baked == code)
                .map_or("", |(_, text)| *text);
            stock.push((hardware, file, text));
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
    fn the_server_sends_the_stock_maps_packed() {
        let hardware = Parts::stock().hardware();
        let print = fingerprint(&hardware);
        let bytes = packed(print).expect("a factory engine's");
        let text = unpack(&bytes).expect("unpacks");
        println!("{} bytes packed, {} as text", bytes.len(), text.len());
        assert!(bytes.len() * 2 < text.len());
        assert!(EngineMaps::from_text(&text, print).is_some());
        assert_eq!(packed(print ^ 1), None);
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
