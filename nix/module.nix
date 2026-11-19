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
          ExecStart = lib.escapeShellArgs [
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
          ];
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
