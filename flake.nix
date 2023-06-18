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

        nativeBuildInputs = [ pkgs.pkg-config ];

        buildInputs = [ pkgs.openssl ];
      };
    in
    with pkgs; {
      packages.${system}.hl =
        writeScriptBin "hl" ''
          RUST_LOG=info \
          CONFIG_PATH=/home/james/dev/my/ynab_updater/settings.toml \
          ${ynab-updater}/bin/hl
        '';

      defaultPackage.${system} = self.packages.${system}.hl;

      devShell.${system} = mkShell {
        buildInputs = [
          rust-analyzer
          rust
          rustup
          pkg-config
          openssl
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
              description = lib.mdDoc "The path of the config file.";
            };
          };

          config = mkIf cfg.enable {
            systemd.user.timers."ynab-updater" = {
              wantedBy = [ "timers.target" ];
              timerConfig = {
                OnBootSec = "10s";
                OnUnitActiveSec = "24h";
                Unit = "ynab-updater.service";
              };
            };

            systemd.user.services."ynab-updater" = {
              environment = {
                RUST_LOG = "info";
                CONFIG_PATH = cfg.configDir;
              };
              serviceConfig = {
                Type = "oneshot";
                ExecStart = "${ynab-updater}/bin/hl";
              };
            };
          };
        };
    };
}
