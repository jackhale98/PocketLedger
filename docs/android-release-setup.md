# Android Release Setup (Play Store)

`build-android.yml` builds a release **AAB** (what Google Play accepts) plus a
release **APK** for sideload testing, both for arm64, on every `v*` tag and on
manual dispatch. Signing is optional: without the secrets below the workflow
warns and produces unsigned artifacts, so forks still build.

## What the workflow does to the generated project

`src-tauri/gen/android` is not committed; CI runs `tauri android init` and then
applies idempotent patches (all under `scripts/ci/`):

| Patch | Why |
|---|---|
| `patch-android-gradle.py` adds a `signingConfigs.release` block reading `gen/android/keystore.properties` | Release signing per [Tauri's guide](https://v2.tauri.app/distribute/sign/android/); skipped when the file is absent |
| `patch-android-main-activity.py` pads the content view with system-bar/cutout/IME insets | targetSdk 35+ enforces edge-to-edge and Android WebView never populates `env(safe-area-inset-*)`, so without this the page sits under the status bar |
| copies `src-tauri/android/release/AndroidManifest.xml` to `gen/android/app/src/release/` | Removes `INTERNET` and `usesCleartextTraffic` from release builds only (the app has no network code); `tauri android dev` keeps them |
| `--config '{"bundle":{"android":{"versionCode":N}}}'` with `N = ANDROID_VERSION_CODE_BASE + run number` | Play rejects a `versionCode` it has seen; Tauri's default derivation repeats when a tag is rebuilt |

Other release-readiness items handled outside CI:

- **16 KB page sizes** (required for apps targeting Android 15+): the NDK is
  pinned to r28 (`NDK_VERSION` in the workflow), and `src-tauri/build.rs` passes
  `-Wl,-z,max-page-size=16384` for Android targets.
- **Target API 36**: Tauri's template sets `compileSdk`/`targetSdk` to 36. Play
  requires new apps and updates to target API 35+ from 31 August 2025 and moves
  the bar up yearly (API 36 expected for the 2026 deadline), so keep the Tauri
  CLI current; the target level comes from the template, not from this repo.

## 1. Create the upload keystore (once)

Do this on your own machine, not in CI. Keep the file and passwords in a
password manager; a lost upload key can be reset through Play support, but it
is a slow process.

```bash
keytool -genkey -v \
  -keystore upload-keystore.jks \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -alias upload
# You will be asked for a keystore password and a key password.
```

## 2. Add the GitHub secrets

Repository → Settings → Secrets and variables → Actions (or an `android-release`
environment if you prefer to gate it; the workflow uses repository secrets by
default).

```bash
base64 -w 0 upload-keystore.jks   # → ANDROID_KEYSTORE_BASE64
```

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | base64 of `upload-keystore.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | keystore password |
| `ANDROID_KEY_ALIAS` | `upload` (or whatever you passed to `-alias`) |
| `ANDROID_KEY_PASSWORD` | key password |

All four must be set; if any is missing the build is unsigned and the job logs
a warning.

## 3. Enrol in Play App Signing

1. Play Console → Create app → PocketHLedger, package `com.pockethledger.app`
   (must match `identifier` in `src-tauri/tauri.conf.json`).
2. Test and release → Setup → App signing. Choose **Use a Google-generated
   key** (recommended). Google keeps the app signing key; the keystore from
   step 1 becomes the *upload* key. Upload its certificate if asked:
   `keytool -export -rfc -keystore upload-keystore.jks -alias upload -file upload_certificate.pem`.
3. Upload the first AAB from a workflow run (Internal testing track is the
   quickest). Play then locks the upload key to that certificate; later
   uploads must be signed with the same keystore.

## 4. Release checklist

- Run `scripts/bump-version.sh X.Y.Z --tag` and push the tag.
- Download `PocketHLedger-android-arm64.aab` from the GitHub release (draft)
  or the `android-release` workflow artifact and upload it to a Play track.
- The `versionCode` is printed in the "Build Android release" step. If Play
  ever reports a lower code than one already uploaded, raise
  `ANDROID_VERSION_CODE_BASE` in `build-android.yml`.
- Data safety form: no data collected, no network access (the release manifest
  has no `INTERNET` permission; the "Verify the release manifest" step proves
  it on every build).

## Verifying an artifact locally

```bash
# Signature
apksigner verify --print-certs PocketHLedger-android-arm64.apk
# Permissions in the built APK (expect no INTERNET)
aapt dump permissions PocketHLedger-android-arm64.apk
# 16 KB alignment of native libs
unzip -o PocketHLedger-android-arm64.apk 'lib/arm64-v8a/*.so' -d /tmp/apk && \
  readelf -lW /tmp/apk/lib/arm64-v8a/libapp_lib.so | grep LOAD   # Align 0x4000
```
