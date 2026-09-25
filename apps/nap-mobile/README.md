# Nap Wallet for iOS and Android

The same wallet as nap.cavebro.win and the browser extension, in a native
shell: `apps/nap-web` built with `--mode mobile` (endpoints on
nap.cavebro.win, see `apps/nap-web/src/origin.js`) inside Tauri 2's web view.
The Rust side is only the shell — no commands, no plugins — so the app is
exactly the wallet: keys made and sealed on the phone, chain reads and signed
transactions to Nap's server. Nap's server answers the app's origins
(`tauri://localhost` on iOS, `http://tauri.localhost` on Android) and no other
cross-origin page (`apps/nap-web/functions/_middleware.js`).

Identifier: `io.infinityteam.napwallet` (both stores). Version: `version` in
`src-tauri/tauri.conf.json`.

## Build and run

```sh
npm install
. scripts/android-env.sh            # Android only: SDK, NDK, JDK, short target dir

npm run ios:sim                     # simulator build → src-tauri/gen/apple/build/arm64-sim/Nap Wallet.app
xcrun simctl install booted "src-tauri/gen/apple/build/arm64-sim/Nap Wallet.app"
xcrun simctl launch booted io.infinityteam.napwallet

npm run android:apk                 # debug APK (WebView debuggable) → gen/android/app/build/outputs/apk/…
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

Debug builds are inspectable: Safari → Develop → Simulator on iOS;
`chrome://inspect` (or `adb forward tcp:9333 localabstract:webview_devtools_remote_<pid>`
and Playwright's `connectOverCDP`) on Android.

Checked 25 Sep 2026: iOS 26.2 simulator (iPhone 17 Pro) and Android 16
emulator — secure context, the chain read (mainnet epoch; testnet wallet
restored, coins named, Zcash address derived and the block scan running in the
app's WebAssembly), safe areas.

## Release

**iOS** (App Store Connect):

1. Apple Developer Program, **enrolled as an organization** (App Review
   guideline 3.1.5: crypto wallets must come from an organization; needs a
   D-U-N-S number).
2. Put the team id in `src-tauri/tauri.conf.json` → `bundle.iOS.developmentTeam`,
   then `npm run ios:release` (archive + export for App Store Connect).
3. Upload the `.ipa` with Xcode's Organizer or Transporter; fill the listing from
   `store/LISTING.md`; screenshots in `store/ios/` (6.9", 1320×2868).

**Android** (Google Play):

1. `sh scripts/make-upload-key.sh` — creates the upload key under
   `~/.nap-release/` and writes `src-tauri/gen/android/keystore.properties`
   (both ignored by git). Back the key up; Play App Signing holds the app
   signing key, this one only uploads.
2. `npm run android:release` → `src-tauri/gen/android/app/build/outputs/bundle/universalRelease/app-universal-release.aab`.
3. Play Console: listing from `store/LISTING.md`, graphics in `store/play/`.

Native libraries are linked for 16 KB pages (`src-tauri/.cargo/config.toml`),
which Play requires for new apps targeting Android 15+.

## Local toolchain note

On this machine the NDK 27.1 wrappers `aarch64-linux-android24-clang` and
`…-clang++` call themselves instead of `clang`, and every arm64 link fails with
"Argument list too long". `scripts/android-env.sh` builds against a shadow NDK
in /tmp with those two wrappers restored, leaving the SDK untouched. The clean
fix is to reinstall NDK 27.1.12297006 from Android Studio's SDK Manager.
