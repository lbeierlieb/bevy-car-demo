//! Puts the icon (`assets/icon/car-sim.ico`, see `src/icon.rs`) into the Windows
//! executable, with mingw's `windres` (the Nix package's cross toolchain has it).
//! Nothing for any other target.

use std::{env, path::PathBuf, process::Command};

const ICON: &str = "assets/icon/car-sim.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=no icon: only the windows-gnu target embeds it");
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join(ICON);
    let script = out.join("icon.rc");
    let icon = icon.to_str().expect("a UTF-8 path").replace('\\', "/");
    std::fs::write(&script, format!("1 ICON \"{icon}\"\n")).expect("writing icon.rc");
    let object = out.join("icon.o");
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH");
        format!("{arch}-w64-mingw32-windres")
    });
    let status = Command::new(&windres)
        .arg(&script)
        .args(["-O", "coff", "-o"])
        .arg(&object)
        .status()
        .unwrap_or_else(|error| panic!("running {windres} (set WINDRES): {error}"));
    assert!(status.success(), "{windres} failed on {}", script.display());
    // Only the game has the icon; an object given to the linker is linked whole, so the
    // resource section goes in although nothing refers to it.
    println!("cargo:rustc-link-arg-bin=car-sim={}", object.display());
}
