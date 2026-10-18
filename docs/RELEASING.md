# Creating FetchX releases

The GitHub Actions release workflow builds Android, Windows, Linux, and macOS
desktop packages, plus CLI binaries for Windows, Linux, and macOS. It attaches
SHA-256 checksum files and creates a **draft** GitHub Release for review.

## Triggering a release

- Push a version tag such as `v1.2.3`.
- Or run **Actions → Release → Run workflow** and enter an app version such as
  `alpha.0.0.1` or `1.2.3`. The workflow creates a `v`-prefixed tag and uses
  the entered value as the Android APK's displayed version. The Android version
  code comes from the workflow run number. The manual run form includes
  switches for Android, Windows, Linux, and macOS builds; all are enabled by
  default. Disabling a platform also skips its CLI binary build (Windows,
  Linux, or macOS). At least one platform must be selected to publish a draft
  release.

Tag-triggered releases always build every platform.

Tag-triggered releases use the tag name as the Android APK's displayed
version. Cargo and Tauri package versions remain defined by their manifests.
Local Android builds default to `0.0.0-dev` unless an `appVersion` Gradle
property is supplied.

## Optional signing secrets

Without Android signing secrets, the release APK is unsigned and is not suitable
for normal installation or future signed updates. Configure all of these
repository secrets to produce a signed APK:

- `ANDROID_KEYSTORE_BASE64`: base64-encoded JKS/PKCS12 keystore
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`
- `ANDROID_STORE_PASSWORD`

For macOS distribution outside development, configure Apple signing and
notarization secrets: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
Windows installers are currently unsigned.

Review and publish the generated draft release only after verifying its
packages, signatures, and checksums.
