## Install from local checkout
- Update installing `flake.nix` to use `ynab-updater.url = "git+file:///home/james/dev/my/ynab_updater"`
- After changes are made to the local checkout run `nix flake lock --update-input ynab-updater` to update the installing flake.lock
- Install new changes via `rebuild --refresh`

## TODO

- [x] add HL
- [x] add YNAB
- [x] add Pushover
- [x] build/run with nix
- [x] package into nix module
- [ ] add logrotate to nix module
