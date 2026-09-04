# DeepScan mobile clients

These are **not** independent file indexers — a phone can't be, by platform design. iOS
forbids background daemons and arbitrary filesystem access outright (no equivalent of
Finder-level hooks exists for a sandboxed app), and Android's broader
`MANAGE_EXTERNAL_STORAGE` access still can't run the ONNX/LanceDB pipeline a phone has no
business running continuously in the background.

Instead, both apps are **remote clients** to a desktop DeepScan engine running on the same
local network: they call the same `/api/search` / `/api/status` / `/api/reveal` JSON
endpoints the web frontend uses (see `docs/ARCHITECTURE.md` and `rust-engine/src/http.rs`),
pointed at the desktop machine's LAN IP instead of `127.0.0.1`. Each app also lets you pick
a local file (Android's Storage Access Framework, iOS's `UIDocumentPickerViewController`)
to search *by* — e.g. drop a photo from your phone to CLIP-search your desktop's indexed
files — without ever indexing the phone itself.

## Android (`android/`)

Kotlin + Jetpack Compose. Builds a real `.apk` via Gradle — see
`.github/workflows/release.yml` at the repo root.

```bash
cd mobile/android && ./gradlew assembleRelease
```

## iOS (`ios/`)

SwiftUI. Produces an Xcode project you can Archive into a `.ipa` — but **only you can
produce a signed, distributable `.ipa`**: that step requires your own Apple Developer
Program membership and signing certificate, which isn't something that can be automated
here. `xcodebuild archive` in CI can produce an unsigned build for local testing; a real
`.ipa` for install-outside-TestFlight needs `xcodebuild -exportArchive` run with your
provisioning profile.
