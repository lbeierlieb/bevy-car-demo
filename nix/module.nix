# The Car Sim multiplayer server as a NixOS service, with its certificate from Let's
# Encrypt (security.acme) and, if wanted, the web version served at the same domain:
#
#   imports = [ car-sim.nixosModules.server ];
#   security.acme = { acceptTerms = true; defaults.email = "you@example.org"; };
#   services.car-sim = {
#     enable = true;
#     domain = "cars.example.org";
#     openFirewall = true;
#     web.enable = true;
#   };
#
# To hear who joins and leaves through a Telegram bot (the token and the chat id each in
# a file, e.g. from sops-nix: `telegram.tokenFile = config.sops.secrets.….path;`):
#
#   services.car-sim.telegram = { tokenFile = "/run/secrets/…"; chatIdFile = "/run/secrets/…"; };
#
# Players log in to accounts, kept with their cars in an SQLite file under
# /var/lib/car-sim (or `database` / `databaseFile`: MySQL or PostgreSQL). The admin makes
# accounts with `sudo car-sim-user add NAME` (also `password NAME`, `remove NAME`, `list`);
# `guests = true` lets in players without one.
#
# The server takes up a renewed certificate by itself (it looks for changed files every
# minute), so renewals don't drop anyone.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.car-sim;
  inherit (lib)
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    types
    ;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  cert = config.security.acme.certs.${cfg.domain};
  # How the server is told its database, if it has one.
  databaseArgs =
    if cfg.databaseFile != null then
      [
        "--database-file"
        "%d/database"
      ]
    else
      lib.optionals (cfg.database != null) [
        "--database"
        cfg.database
      ];
  # The admin's account command: as the service's user with a database URL (an SQLite file
  # stays the service's), as root with a URL file (it may be root's alone).
  userCommand = pkgs.writeShellScriptBin "car-sim-user" (
    if cfg.databaseFile != null then
      ''
        exec ${lib.getExe cfg.package} --database-file ${lib.escapeShellArg cfg.databaseFile} user "$@"
      ''
    else
      ''
        exec ${pkgs.util-linux}/bin/runuser -u car-sim -- ${lib.getExe cfg.package} --database ${lib.escapeShellArg (toString cfg.database)} user "$@"
      ''
  );
  hasDatabase = cfg.database != null || cfg.databaseFile != null;
in
{
  options.services.car-sim = {
    enable = mkEnableOption "the Car Sim multiplayer server";

    package = mkOption {
      type = types.package;
      default = packages.server;
      defaultText = lib.literalExpression "car-sim.packages.\${system}.server";
      description = "The server package.";
    };

    domain = mkOption {
      type = types.str;
      example = "cars.example.org";
      description = ''
        The server's domain: players join it by this name, and its certificate is for it.
        It needs a DNS record pointing at this machine.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 4433;
      description = "The UDP port players join on (WebTransport).";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Open the UDP port for players, and TCP 80 for getting the certificate (and 443
        with the web version).
      '';
    };

    traffic = mkOption {
      type = types.ints.between 0 60;
      default = 20;
      description = "How many traffic cars drive around each player.";
    };

    maxPlayers = mkOption {
      type = types.ints.positive;
      default = 32;
      description = "The most players at once.";
    };

    mapPlayers = mkOption {
      type = types.enum [
        "self"
        "near"
        "all"
      ];
      default = "near";
      description = ''
        Who a player sees of the others on their map: nobody (`self`), the ones within
        {option}`services.car-sim.mapNear` metres (`near`) or everyone (`all`).
      '';
    };

    mapNear = mkOption {
      type = types.ints.unsigned;
      default = 200;
      description = "How near other players are shown on the map with `mapPlayers = \"near\"` (m).";
    };

    chat = mkOption {
      type = types.bool;
      default = true;
      description = "Whether players can chat (T in the game).";
    };

    nameTags = mkOption {
      type = types.bool;
      default = true;
      description = "Whether the players' cars show their names.";
    };

    shops = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = lib.literalExpression ''"''${car-sim}/assets/shops.cfg"'';
      description = ''
        A file saying where the shops on the map are (lines of kind, x, z and name, as in
        the repo's assets/shops.cfg; their markers go on the nearest paved road). Null:
        no shops.
      '';
    };

    events = mkOption {
      type = types.nullOr types.path;
      default = null;
      example = lib.literalExpression ''"''${car-sim}/assets/events.cfg"'';
      description = ''
        A file saying where the events on the map are (lines of kind, x, z and name, as in
        the repo's assets/events.cfg: a car hunt, races with their routes under them; their
        markers go exactly there). Null: no events. The server checks the races' routes
        against the map when it starts, and won't start if one is wrong.
      '';
    };

    spawn = mkOption {
      type = types.nullOr (
        types.submodule {
          options = {
            x = mkOption {
              type = types.number;
              description = "East (m on the map, as the gizmos (1) and the map (M) show it).";
            };
            z = mkOption {
              type = types.number;
              description = "South (m on the map).";
            };
            heading = mkOption {
              type = types.nullOr types.number;
              default = null;
              description = ''
                Compass degrees (0 north, 90 east) to face along the road there. Null: the
                way with more road behind.
              '';
            };
          };
        }
      );
      default = null;
      example = {
        x = -2984.5;
        z = -2007.2;
        heading = 180;
      };
      description = ''
        Where players start: they're placed on the paved road nearest that map point. Null:
        the game's default, in Gerach.
      '';
    };

    database = mkOption {
      type = types.nullOr types.str;
      default = "sqlite:///var/lib/car-sim/car-sim.db";
      example = "postgres://car-sim@localhost/car-sim";
      description = ''
        Where accounts and what players own are kept: an SQLite file (under
        /var/lib/car-sim, the service's state directory), or a MySQL/MariaDB
        (`mysql://user:password@host/db`) or PostgreSQL (`postgres://…`) database, which
        several servers may share (a player plays on one at a time). Null: no accounts,
        everyone plays by the name they give and nothing is kept. Make accounts with
        `sudo car-sim-user add NAME`.
      '';
    };

    databaseFile = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "/run/secrets/car-sim-database";
      description = ''
        A file holding the database's URL instead of {option}`services.car-sim.database`
        (when it has a password: kept out of the Nix store, read as a systemd credential).
      '';
    };

    guests = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Whether players without an account may play too, by a name that isn't an
        account's and without a password (nothing of theirs is kept).
      '';
    };

    telegram = {
      tokenFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/run/secrets/car-sim-telegram";
        description = ''
          A file holding a Telegram bot's token (as a string, so the secret stays out of
          the Nix store; e.g. a sops-nix secret's path). If set, the bot tells
          the chat in {option}`services.car-sim.telegram.chatIdFile` who joins and leaves the server.
          The service reads it as a systemd credential, so the file may stay root's.
        '';
      };

      chatIdFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/run/secrets/car-sim-telegram-chat";
        description = ''
          A file holding the chat the bot writes to (kept secret like the token, read as a
          systemd credential too): a user's, group's or channel's id, or a public
          channel's `@name`. A user must have written to the bot first, and the bot must
          be in a group or channel (an admin in a channel). Write to the bot and open
          `https://api.telegram.org/bot<token>/getUpdates` to find your id.
        '';
      };
    };

    web = {
      enable = mkEnableOption "serving the web version at https://<domain> with nginx (it joins this server)";

      package = mkOption {
        type = types.package;
        default = packages.web;
        defaultText = lib.literalExpression "car-sim.packages.\${system}.web";
        description = "The web build.";
      };
    };
  };

  config = mkIf cfg.enable (mkMerge [
    {
      assertions = [
        {
          assertion = (cfg.telegram.tokenFile != null) == (cfg.telegram.chatIdFile != null);
          message = "services.car-sim.telegram.tokenFile and chatIdFile go together";
        }
        {
          assertion = cfg.guests -> hasDatabase;
          message = "services.car-sim.guests needs a database (without one, everyone's a guest)";
        }
      ];

      environment.systemPackages = lib.optional hasDatabase userCommand;

      # A user of its own (not `DynamicUser`: that can't share its name with the
      # certificate's group).
      users.users.car-sim = {
        isSystemUser = true;
        group = "car-sim";
      };
      users.groups.car-sim = { };

      systemd.services.car-sim = {
        description = "Car Sim multiplayer server";
        wantedBy = [ "multi-user.target" ];
        wants = [
          "network-online.target"
          "acme-finished-${cfg.domain}.target"
        ];
        after = [
          "network-online.target"
          "acme-finished-${cfg.domain}.target"
        ];
        serviceConfig = {
          ExecStart = lib.escapeShellArgs (
            [
              (lib.getExe cfg.package)
              "--port"
              cfg.port
              "--cert"
              "${cert.directory}/fullchain.pem"
              "--key"
              "${cert.directory}/key.pem"
              "--traffic"
              cfg.traffic
              "--max-players"
              cfg.maxPlayers
              "--map-players"
              cfg.mapPlayers
              "--map-near"
              cfg.mapNear
              "--chat"
              (if cfg.chat then "on" else "off")
              "--name-tags"
              (if cfg.nameTags then "on" else "off")
            ]
            ++ lib.optionals (cfg.shops != null) [
              "--shops"
              "${cfg.shops}"
            ]
            ++ lib.optionals (cfg.events != null) [
              "--events"
              "${cfg.events}"
            ]
            ++ lib.optionals (cfg.spawn != null) [
              "--spawn"
              (lib.concatMapStringsSep " " toString (
                [
                  cfg.spawn.x
                  cfg.spawn.z
                ]
                ++ lib.optional (cfg.spawn.heading != null) cfg.spawn.heading
              ))
            ]
            ++ databaseArgs
            ++ lib.optionals hasDatabase [
              "--guests"
              (if cfg.guests then "on" else "off")
            ]
            # The credentials' copies (%d: the service's credentials directory).
            ++ lib.optionals (cfg.telegram.tokenFile != null) [
              "--telegram-token-file"
              "%d/telegram-token"
              "--telegram-chat-file"
              "%d/telegram-chat"
            ]
          );
          LoadCredential =
            lib.optionals (cfg.telegram.tokenFile != null) [
              "telegram-token:${cfg.telegram.tokenFile}"
              "telegram-chat:${cfg.telegram.chatIdFile}"
            ]
            ++ lib.optional (cfg.databaseFile != null) "database:${cfg.databaseFile}";
          # The SQLite file's home (/var/lib/car-sim).
          StateDirectory = "car-sim";
          StateDirectoryMode = "0750";
          Restart = "on-failure";
          RestartSec = 5;
          User = "car-sim";
          Group = "car-sim";
          # To read the certificate.
          SupplementaryGroups = [ cert.group ];
          # A port under 1024 needs the capability; nothing else does.
          AmbientCapabilities = lib.optional (cfg.port < 1024) "CAP_NET_BIND_SERVICE";
          CapabilityBoundingSet = lib.optional (cfg.port < 1024) "CAP_NET_BIND_SERVICE";
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          PrivateDevices = true;
          ProtectKernelTunables = true;
          ProtectKernelModules = true;
          ProtectKernelLogs = true;
          ProtectControlGroups = true;
          ProtectClock = true;
          ProtectHostname = true;
          RestrictAddressFamilies = [
            "AF_INET"
            "AF_INET6"
            "AF_UNIX"
          ];
          RestrictNamespaces = true;
          RestrictRealtime = true;
          LockPersonality = true;
          SystemCallArchitectures = "native";
          SystemCallFilter = [ "@system-service" ];
        };
      };

      networking.firewall.allowedUDPPorts = mkIf cfg.openFirewall [ cfg.port ];
    }

    # The certificate alone: ACME's own little web server answers the challenge.
    (mkIf (!cfg.web.enable) {
      security.acme.certs.${cfg.domain} = {
        listenHTTP = ":80";
        group = "car-sim";
      };
      networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ 80 ];
    })

    # With the web version: nginx gets the certificate and serves the page.
    (mkIf cfg.web.enable {
      services.nginx = {
        enable = true;
        virtualHosts.${cfg.domain} = {
          enableACME = true;
          forceSSL = true;
          root = cfg.web.package;
        };
      };
      networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [
        80
        443
      ];
    })
  ]);
}
