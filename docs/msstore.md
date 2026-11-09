# Microsoft Store build

The Store listing is **Dispatched Explorer** (Store ID `9NNTH1L8LCM4`), submitted as an MSIX bundle for x64 and ARM64.
Microsoft signs the package after certification, so no certificate is involved. The direct downloads on GitHub are
built separately and are not covered by that signature.

The Store build comes from the same code as the direct download. It differs in three ways:

- **No updater.** Built with `--no-default-features --features custom-protocol`, and `tauri.msstore.conf.json` drops
  the updater capability. The app reports `updates: false`, and Settings says "Updates come from the Microsoft Store."
- **Its own data.** Windows redirects a Store app's writes to `%APPDATA%` into its package folder, under
  `%LOCALAPPDATA%\Packages\HorizonTechd.o.o.DispatchedExplorer_x28vjc8ra1q9g\LocalCache`. The Store app therefore does
  not share saved connections or the activity log with the `dispatched` CLI. The direct download does.
- **Version number.** The first part of a Store version can't be 0 and the fourth must be 0, so the Store version is
  the app version with one added to the major part: 0.1.0 ships as 1.1.0.0, and 1.0.0 would ship as 2.0.0.0. The app's
  Settings still shows the app version.

`app/src-tauri/msstore/Package.appxmanifest` holds the package identity from Partner Center (Product identity). The
values are case-sensitive and must match exactly.

## Build

Run the **Microsoft Store** workflow from the Actions tab. It builds `Dispatched_<store version>.msixbundle`
(artifact `microsoft-store-package`), installs a test copy of the x64 package on the Windows runner, launches it, and
saves a screenshot (artifact `microsoft-store-checks`). The ARM64 package is built but not launched.

The Windows App Certification Kit is not part of the job: on GitHub's Windows Server runner it fails while writing its
report and marks the tests failed with no reason. To run it, use Windows 10 or 11.

On a Windows machine, `scripts/msstore.ps1` builds the same bundle into `msstore/`.

## Submit

In Partner Center, open the app and start a submission:

1. **Pricing and availability, Properties, Age ratings:** fill these in; the category is Developer tools.
2. **Packages:** upload the `.msixbundle`. It must be unzipped from the workflow artifact first.
3. **Store listings:** reuse the App Store text, and add at least one Windows screenshot.
4. **Submission options:** the package uses the restricted `runFullTrust` capability, so give the reason:

   > Dispatched Explorer is a Win32 desktop app built on Tauri and WebView2, packaged as MSIX. It needs full trust to
   > run as a desktop process, to open AMQP and HTTPS connections to the user's Azure Service Bus namespaces, to
   > receive the loopback redirect of the Microsoft Entra sign-in, and to keep sign-ins in Windows Credential Manager.

A new version needs a higher app version than the last submission, because the Store version is derived from it.
