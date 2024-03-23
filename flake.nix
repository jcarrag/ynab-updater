{
  description = "A YNAB updater";

  inputs.rustOverlay.url = "github:oxalica/rust-overlay";

  outputs = { self, unstable, rustOverlay }:
    let
      system = "x86_64-linux";

      pname = "ynab-updater";

      pkgs = import unstable { inherit system; overlays = [ rustOverlay.overlay ]; };

      rust = pkgs.rust-bin.nightly.latest.default.override {
        extensions = [
          "rust-src"
          "clippy-preview"
          "rustfmt-preview"
        ];
      };

      rustPlatform = pkgs.makeRustPlatform {
        cargo = rust;
        rustc = rust;
      };

      ynab-updater = rustPlatform.buildRustPackage {
        inherit pname;

        version = "0.0.1";

        src = ./.;

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        nativeBuildInputs = [ pkgs.pkg-config ];

        buildInputs = [
          pkgs.openssl
          pkgs.systemd
        ];
      };
    in
    with pkgs; {
      packages.${system} = {
        hl = writeShellScriptBin "hl" ''
          RUST_LOG=info \
          RUST_BACKTRACE=1 \
          YNAB_CONFIG_PATH=''${YNAB_CONFIG_PATH:-/home/james/dev/my/ynab_updater} \
          ${ynab-updater}/bin/hl
        '';
        saxo = writeShellScriptBin "saxo" ''
          RUST_LOG=info \
          RUST_BACKTRACE=1 \
          YNAB_TAILSCALE_IP=$(${pkgs.tailscale}/bin/tailscale ip --4) \
          YNAB_CONFIG_PATH=''${YNAB_CONFIG_PATH:-/home/james/dev/my/ynab_updater} \
          ${ynab-updater}/bin/saxo
        '';
      };

      devShell.${system} = mkShell {
        buildInputs = [
          rust-analyzer
          rust
          rustup
          pkg-config
          openssl
          systemd
        ];
      };

      nixosModules.ynab-updater = { config, pkgs, ... }:
        with lib; with lib.types;
        let
          cfg = config.programs.ynab-updater;
        in
        {
          options.programs.ynab-updater = {
            enable = mkEnableOption "Enable the YNAB updater service.";
            configDir = mkOption {
              type = types.str;
              description = lib.mdDoc "The directory of the config file & cache.";
            };
          };

          config = mkIf cfg.enable {
            systemd.user.timers."ynab-updater-hl" = {
              wantedBy = [ "timers.target" ];
              timerConfig = {
                OnBootSec = "10s";
                OnUnitActiveSec = "24h";
                Unit = "ynab-updater-hl.service";
              };
            };
            systemd.user.services."ynab-updater-hl" = {
              environment = {
                RUST_LOG = "info";
                YNAB_CONFIG_PATH = cfg.configDir;
              };
              serviceConfig = {
                Type = "oneshot";
                ExecStart = "${self.packages.${system}.hl}/bin/hl";
              };
            };

            systemd.user.timers."ynab-updater-saxo" = {
              wantedBy = [ "timers.target" ];
              timerConfig = {
                OnBootSec = "10s";
                # 55m since the refresh_token duration is 1h
                # - so we want to refresh it before it expires
                OnUnitActiveSec = "55m";
                Unit = "ynab-updater-saxo.service";
              };
            };
            systemd.user.services."ynab-updater-saxo" = {
              environment = {
                RUST_LOG = "info";
                YNAB_CONFIG_PATH = cfg.configDir;
              };
              serviceConfig = {
                Type = "oneshot";
                # Necessary otherwise:
                # > Mar 22 09:54:02 xps systemd[1841]: ynab-updater-saxo.service: Got notification message from PID 283457, but reception only permitted for main PID 283432
                # https://github.com/systemd/systemd/issues/24516#issuecomment-1233032190
                NotifyAccess = "all";
                FileDescriptorStoreMax = 16;
                FileDescriptorStorePreserve = "yes";
                ExecStart = "${self.packages.${system}.saxo}/bin/saxo";
              };
            };
          };
        };
    };
}
