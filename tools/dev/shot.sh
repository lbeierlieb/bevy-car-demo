#!/usr/bin/env bash
# usage: shot.sh out.png [frames] [keys]   (after unstrip.sh; keys: w a s d c t f k 3, space)
# Renders headless with Xvfb and Mesa's software Vulkan (lavapipe), both from the nixpkgs
# pinned in flake.lock (fetched on first use).
cd /repo
REV=$(grep -A6 '"nixpkgs": {' flake.lock | grep '"rev"' | head -1 | cut -d'"' -f4)
pkg() { nix build --no-link --print-out-paths "github:NixOS/nixpkgs/$REV#$1"; }
MESA=$(pkg mesa)
XVFB=$(pkg xvfb-run)
export VK_ICD_FILENAMES=$(ls $MESA/share/vulkan/icd.d/lvp_icd*.json | head -1)
SHOT="$1" FRAMES="${2:-120}" KEYS="$3" nix develop -c $XVFB/bin/xvfb-run -a -s "-screen 0 1280x720x24" env CARGO_TARGET_DIR=/repo/target/sandbox cargo run 2>&1 | grep -vE "^\s*$" | grep -iE "error|panic|warn|^WHEEL" | grep -v "Git tree"
