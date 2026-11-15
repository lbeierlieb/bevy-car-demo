#!/usr/bin/env bash
# usage: shot.sh out.png [frames] [keys]   (after unstrip.sh; keys: w a s d c t f, space)
# Renders headless with Xvfb and Mesa's software Vulkan (lavapipe). The store paths are from
# the sandbox VM; elsewhere find them with `nix build nixpkgs#mesa nixpkgs#xvfb-run --print-out-paths`.
MESA=/nix/store/jg8f97fhfmkpccrm3lsxlavkivpjbs72-mesa-26.2.5
XVFB=/nix/store/kcz5yrhdj95sgh31hzp5k8zmf8lvzvi4-xvfb-run-1+g87f6705
cd /repo
export VK_ICD_FILENAMES=$(ls $MESA/share/vulkan/icd.d/lvp_icd*.json | head -1)
SHOT="$1" FRAMES="${2:-120}" KEYS="$3" nix develop -c $XVFB/bin/xvfb-run -a -s "-screen 0 1280x720x24" env CARGO_TARGET_DIR=/repo/target/sandbox cargo run 2>&1 | grep -vE "^\s*$" | grep -iE "error|panic|warn|^WHEEL" | grep -v "Git tree"
