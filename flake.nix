{
  description = "Car simulation demo: rigid-body chassis, double A-arm suspension, raycast tires (Bevy)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      # Everything this flake provides, as { <system> = { packages, apps, checks, devShells }; }.
      # Evaluated once per system and shared by all outputs, so nixpkgs is imported only once.
      bySystem = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          lib = pkgs.lib;

          toolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
            targets = [ "wasm32-unknown-unknown" ];
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchain);

          cargoToml = lib.importTOML ./Cargo.toml;
          pname = cargoToml.package.name;
          version = cargoToml.package.version;

          # Must match the `wasm-bindgen` version pinned in Cargo.lock.
          wasm-bindgen-cli = pkgs.wasm-bindgen-cli_0_2_130;

          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./assets
              ./src
              ./web
              ./build.rs
            ];
          };

          # Libraries Bevy loads at runtime on Linux (windowing + GPU).
          runtimeLibs = lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs;
            [
              vulkan-loader
              libGL
              libxkbcommon
              wayland
              libx11
              libxcursor
              libxi
              libxrandr
            ]
          );
          buildLibs = lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs;
            [
              udev
              alsa-lib
            ]
          );

          # Each target builds its dependencies once (`*Deps`); its package and clippy
          # check reuse them, so Bevy isn't compiled again for every derivation.
          nativeArgs = {
            inherit pname version src;
            strictDeps = true;
            doCheck = false;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = buildLibs ++ runtimeLibs;
          };
          nativeDeps = craneLib.buildDepsOnly nativeArgs;

          wasmArgs = {
            pname = "${pname}-wasm";
            inherit version src;
            strictDeps = true;
            doCheck = false;
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            CARGO_PROFILE = "wasm-release";
          };
          wasmDeps = craneLib.buildDepsOnly wasmArgs;

          # The game alone (`nix run`): the editors are for the dev shell, the server is a
          # package of its own. Each binary is a full LTO link of Bevy, so building them all
          # took about four times as long.
          native = craneLib.buildPackage (
            nativeArgs
            // {
              cargoArtifacts = nativeDeps;
              cargoExtraArgs = "--locked --bin ${pname}";
              nativeBuildInputs =
                nativeArgs.nativeBuildInputs ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.makeWrapper ];
              postFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                wrapProgram $out/bin/${pname} \
                  --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
              '';
              meta.mainProgram = pname;
            }
          );

          # The dedicated multiplayer server, headless.
          server = craneLib.buildPackage (
            nativeArgs
            // {
              pname = "${pname}-server";
              cargoArtifacts = nativeDeps;
              cargoExtraArgs = "--locked --bin server";
              meta = {
                description = "Car Sim multiplayer server";
                mainProgram = "server";
              };
            }
          );

          wasm = craneLib.mkCargoDerivation (
            wasmArgs
            // {
              cargoArtifacts = wasmDeps;
              doInstallCargoArtifacts = false;
              nativeBuildInputs = [
                wasm-bindgen-cli
                pkgs.binaryen
                pkgs.removeReferencesTo
              ];
              buildPhaseCargoCommand = "cargoWithProfile build --locked";
              installPhaseCommand = ''
                mkdir -p $out
                wasm-bindgen --target web --no-typescript \
                  --out-dir $out --out-name ${pname} \
                  target/wasm32-unknown-unknown/wasm-release/${pname}.wasm
                wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
                  -o $out/${pname}_bg.wasm $out/${pname}_bg.wasm
                cp web/index.html $out/
                # Panic messages name source files of the toolchain and the crates: as
                # store paths they'd make all of those part of the web build's closure
                # (and of a server serving it).
                remove-references-to -t ${toolchain} -t "$cargoVendorDir" $out/*
              '';
              dontFixup = true;
            }
          );

          # The game for Windows (x86_64), cross-compiled with mingw: `car-sim.exe` stands
          # alone (the map, fonts, shops and events are compiled in; no DLLs), and
          # `car-sim-windows.zip` holds it with a README and the font's licence. A toolchain
          # of its own, so the other packages' dependencies aren't built again for it.
          mingw = pkgs.pkgsCross.mingwW64;
          mingwBin = "${mingw.stdenv.cc}/bin/${mingw.stdenv.cc.targetPrefix}";
          windowsCraneLib = (crane.mkLib pkgs).overrideToolchain (
            p: p.rust-bin.stable.latest.minimal.override { targets = [ "x86_64-pc-windows-gnu" ]; }
          );
          windowsArgs = {
            pname = "${pname}-windows";
            inherit version src;
            strictDeps = true;
            doCheck = false;
            depsBuildBuild = [ mingw.stdenv.cc ];
            CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
            CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${mingwBin}cc";
            # Rust links `libpthread.a` statically (no libwinpthread DLL to ship).
            CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${mingw.windows.pthreads}/lib";
            # For the C in ring and SQLite, and the icon (`build.rs`).
            CC_x86_64_pc_windows_gnu = "${mingwBin}cc";
            AR_x86_64_pc_windows_gnu = "${mingwBin}ar";
            WINDRES = "${mingwBin}windres";
          };
          windowsDeps = windowsCraneLib.buildDepsOnly windowsArgs;
          windowsReadme = pkgs.writeText "README.txt" ''
            Car Sim for Windows (64-bit)

            Start car-sim.exe. It isn't signed, so Windows may warn the first time: "More
            info", then "Run anyway".

            It needs a graphics card with DirectX 12 or Vulkan. Steering wheels, pedals and
            gamepads work as controls (Esc, Settings, Controls); force feedback is only on
            Linux for now.

            Settings and playing alone's garage are kept in %APPDATA%\car-sim, worked out
            engine maps in %LOCALAPPDATA%\car-sim (can be deleted).

            The menus' font is Ubuntu, under the Ubuntu Font Licence
            (UBUNTU-FONT-LICENCE.txt).
          '';
          windows = windowsCraneLib.buildPackage (
            windowsArgs
            // {
              cargoArtifacts = windowsDeps;
              cargoExtraArgs = "--locked --bin ${pname}";
              nativeBuildInputs = [ pkgs.zip ];
              # The host's strip doesn't know Windows executables.
              dontStrip = true;
              postInstall = ''
                ${mingwBin}strip $out/bin/${pname}.exe
                mkdir -p zip/${pname}
                cp $out/bin/${pname}.exe zip/${pname}/
                cp ${windowsReadme} zip/${pname}/README.txt
                cp ${./assets/fonts/LICENCE.txt} zip/${pname}/UBUNTU-FONT-LICENCE.txt
                find zip -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
                (cd zip && zip -9 -X -r $out/${pname}-windows.zip ${pname})
              '';
              meta.description = "Car Sim for Windows (x86_64), cross-compiled";
            }
          );

          # Dev shell only: `cargo run` becomes an optimized build with quick incremental
          # rebuilds (the `release-dev` profile). Other cargo commands pass through.
          cargo-dev = pkgs.writeShellScriptBin "cargo" ''
            if [ "''${1:-}" = "run" ]; then
              shift
              exec ${toolchain}/bin/cargo run --profile release-dev "$@"
            fi
            exec ${toolchain}/bin/cargo "$@"
          '';

          # `cargo clippy` for one target, failing on any warning.
          clippy =
            args: deps:
            craneLib.cargoClippy (
              args
              // {
                cargoArtifacts = deps;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );

          # Lint the Nix files with a tool that exits non-zero on problems.
          nixLint =
            name: tool: command:
            pkgs.runCommand name { nativeBuildInputs = [ tool ]; } ''
              ${command} ${./flake.nix} ${./nix/module.nix}
              touch $out
            '';

          # The server's NixOS module evaluated with and without the web version: what the
          # service runs and how the certificate is got.
          moduleCheck =
            let
              evaluate =
                web:
                (nixpkgs.lib.nixosSystem {
                  inherit system;
                  modules = [
                    self.nixosModules.server
                    {
                      boot.isContainer = true;
                      system.stateVersion = "25.11";
                      security.acme = {
                        acceptTerms = true;
                        defaults.email = "admin@example.org";
                      };
                      services.car-sim = {
                        enable = true;
                        domain = "cars.example.org";
                        openFirewall = true;
                        web.enable = web;
                        # Tried along with the web version.
                        shops = lib.mkIf web ./assets/shops.cfg;
                        # Tried along with the web version.
                        spawn = lib.mkIf web {
                          x = -1476;
                          z = -1213.5;
                          heading = 90;
                        };
                        # Tried along with the web version.
                        guests = web;
                        # Tried along with the web version.
                        telegram = lib.mkIf web {
                          tokenFile = "/run/secrets/car-sim-telegram";
                          chatIdFile = "/run/secrets/car-sim-telegram-chat";
                        };
                      };
                    }
                  ];
                }).config;
              summary = config: {
                user = config.systemd.services.car-sim.serviceConfig.User;
                dynamicUser = config.systemd.services.car-sim.serviceConfig.DynamicUser or false;
                exec = config.systemd.services.car-sim.serviceConfig.ExecStart;
                credentials = config.systemd.services.car-sim.serviceConfig.LoadCredential;
                state = config.systemd.services.car-sim.serviceConfig.StateDirectory;
                packages = map (p: p.name) (
                  builtins.filter (p: lib.hasPrefix "car-sim-user" p.name) config.environment.systemPackages
                );
                groups = config.systemd.services.car-sim.serviceConfig.SupplementaryGroups;
                certGroup = config.security.acme.certs."cars.example.org".group;
                tcp = config.networking.firewall.allowedTCPPorts;
                udp = config.networking.firewall.allowedUDPPorts;
              };
            in
            pkgs.writeText "nixos-module.json" (
              builtins.toJSON {
                alone = summary (evaluate false);
                web = summary (evaluate true);
              }
            );

          # `nix run .#serve-wasm` serves the wasm build on http://localhost:8080
          serve-wasm = pkgs.writeShellApplication {
            name = "serve-wasm";
            runtimeInputs = [ pkgs.python3 ];
            text = ''
              echo "Serving on http://localhost:8080"
              python3 -m http.server 8080 --directory ${wasm}
            '';
          };
        in
        {
          packages = {
            default = native;
            web = wasm;
            inherit
              native
              wasm
              server
              serve-wasm
              ;
          }
          // lib.optionalAttrs (system == "x86_64-linux") {
            inherit windows;
          };

          apps = {
            default = {
              type = "app";
              program = lib.getExe native;
              meta.description = "Run the car demo";
            };
            serve-wasm = {
              type = "app";
              program = lib.getExe serve-wasm;
              meta.description = "Serve the web build on http://localhost:8080";
            };
          };

          checks = {
            # Packaging steps clippy doesn't exercise: linking, wasm-bindgen, wasm-opt.
            inherit native wasm;
            fmt = craneLib.cargoFmt { inherit pname version src; };
            clippy-native = clippy nativeArgs nativeDeps;
            clippy-wasm = clippy wasmArgs wasmDeps;
            inherit server;
          }
          // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            nixos-module = moduleCheck;
          }
          // {
            nixfmt = nixLint "nixfmt" pkgs.nixfmt "nixfmt --check";
            deadnix = nixLint "deadnix" pkgs.deadnix "deadnix --fail";
          };

          devShells.default = pkgs.mkShell {
            packages = [
              toolchain
              wasm-bindgen-cli
              pkgs.binaryen
              pkgs.pkg-config
              pkgs.python3
              pkgs.nixfmt
              pkgs.deadnix
            ]
            ++ buildLibs
            ++ runtimeLibs;
            LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
            shellHook = ''
              export PATH=${cargo-dev}/bin:$PATH
            '';
          };
        }
      );
    in
    {
      packages = forAllSystems (system: bySystem.${system}.packages);
      apps = forAllSystems (system: bySystem.${system}.apps);
      checks = forAllSystems (system: bySystem.${system}.checks);
      devShells = forAllSystems (system: bySystem.${system}.devShells);

      # The multiplayer server as a NixOS service (see nix/module.nix).
      nixosModules.server = import ./nix/module.nix self;
    };
}
