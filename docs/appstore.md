# Mac App Store build

The direct download (GitHub releases, dispatchedapp.com) and the Mac App Store build come from the same code.
The store build differs in four ways:

- **Sandboxed.** `app/src-tauri/Entitlements.appstore.plist`: network client (AMQP, management, ARM, Entra),
  network server (the loopback redirect of the browser sign-in), and files the user picks in an open or save
  dialog. Nothing else.
- **No updater.** Built with `--no-default-features --features custom-protocol`, which leaves out
  `tauri-plugin-updater`; `tauri.appstore.conf.json` drops the updater capability and updater artifacts. The app
  reports `updates: false`, so it never checks and Settings says "Updates come from the App Store."
- **Its own data.** Inside the sandbox, the profile database, activity log and settings live in the app's
  container (`~/Library/Containers/io.horizon-tech.dispatched/`), so the store app does not share saved
  connections or the activity log with the `dispatched` CLI. The direct download does.
- **Opening the browser and Finder** goes through `NSWorkspace` on every macOS build (`app/src-tauri/src/workspace.rs`),
  which works inside the sandbox.

`Info.plist` declares `ITSAppUsesNonExemptEncryption = false` (only the system's standard TLS), so uploads don't
stop at the export-compliance question.

## Build and upload

One-time, per Mac:

1. Certificates of team N9H2M3339M in the login keychain: **Apple Distribution** and **Mac Installer
   Distribution** ("3rd Party Mac Developer Installer").
2. A **Mac App Store** provisioning profile for `io.horizon-tech.dispatched`, saved as
   `appstore/Dispatched_Mac_App_Store.provisionprofile` (git-ignored).
3. The App Store Connect API key at `~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8`.

Then, from the repository root (check free space first: `df -h /System/Volumes/Data`):

```sh
APPSTORE_API_KEY_ID=<key id> APPSTORE_API_ISSUER=<issuer id> scripts/appstore.sh <build-number> --upload
```

The build number must grow with every upload of the same version. Without `--upload` the script only builds the
signed `appstore/Dispatched_<version>_<build>.pkg`. The script refuses to package an app that isn't sandboxed.
