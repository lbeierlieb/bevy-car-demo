---
name: windows-build
description: Building, packaging and trying the Windows (mingw) build of the game, its icon and Windows storage paths. Use when touching the windows package, icon.rs, build.rs or storage on Windows.
---

# Windows build (asked for by the user, 2026-11-27)

`nix build 'git+file:///repo#windows'` (x86_64-linux only; not in `checks`, at the
user's wish): the game cross-compiled for `x86_64-pc-windows-gnu` with
`pkgsCross.mingwW64` (a toolchain of its own, `windowsCraneLib`, so the other packages'
dependencies aren't rebuilt; ~25 min from scratch here). Out: `bin/car-sim.exe` (~95
MB, everything compiled in; imports only system DLLs, pthreads linked statically) and
`car-sim-windows.zip` (the exe, a README, the Ubuntu font's licence). No console window
(`windows_subsystem` in `main.rs`, release only). **Icon**: a steering wheel drawn by
`src/icon.rs` (`icon::wheel(size)`, RGBA): the window's icon on every native app
(`WindowIconPlugin` in `default_plugins`, through `bevy::winit::WINIT_WINDOWS`; `winit`
is a direct dependency for `Icon`), and `assets/icon/car-sim.ico` (PNG entries 16–256),
written by the ignored test `write_icon`; `the_icon_file_is_current` fails after a
drawing change until it's re-run. `build.rs` puts the `.ico` into the exe with mingw's
`windres` (`WINDRES` in the package) for the windows-gnu target only. Storage on Windows:
`storage::game_dir` (`%APPDATA%\car-sim` config and data, `%LOCALAPPDATA%\car-sim` cache;
before, nothing had a folder there, and playing alone couldn't open its database).
Tried under wine 11 (from the pinned nixpkgs, lavapipe as its Vulkan via
`VK_ICD_FILENAMES`, Xvfb): it starts and renders the quick start with the icon in the
title bar, but only in wine's virtual desktop (`wine explorer /desktop=game,1280x720
car-sim.exe`); on the bare Xvfb screen winit panics listing display modes (wine's modes
lack fields). Keys from xdotool never reached it (Xvfb's focus), so playing alone wasn't
tried there. Never run on real Windows yet. With the GUI subsystem the log only shows
under wine.
