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
- [ ] switch to using [nix-crane](https://github.com/ipetkov/crane)

## Security plan: replace HL creds in settings.conf with:
  1. Create authenticator service that's long lived and on startup requests password via [tsnet](https://pkg.go.dev/tailscale.com@main/tsnet) `Funnel API`
    - Written in Go
    - Use IPC to communicate password to fetcher services
    - TS Auth Keys are limited to 90 days, use [OAuth client](https://tailscale.com/kb/1215/oauth-clients) to generate Auth Key as needed - do OAuth login via Pushover
    - Use linux file permissions to only permit ynab-updater-{hl,saxo} from accessing the unix socket (that the http server runs on)

  2. check for a memfd backed fd (via the fdstore) that contains a saved decryption key
  3. if it exists use it to descrypt the HL creds file
  4. if it doesn't exist start a `https://TAILSCALE_HOSTNAME:xxx/decrypt` route that serves a form that accepts the decryption key (over TS' TLS)
     a. also use `Authorization: xxx` header? Could use webauthn (incl. decryption key in header?) / basic-auth (incl. email/pass in header)
  5. send a pushover notification with the url to user's phone
  6. when the user enters the decryption key save it to memfd + notify the fdstore with the memfd fd (return 400 if incorrect decryption key provided)
  7. decrypt the creds + update HL balance
