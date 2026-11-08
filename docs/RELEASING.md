# Creating FetchX releases

The GitHub Actions release workflow builds Android, Windows, Linux, and macOS
desktop packages, plus CLI binaries for Windows, Linux, and macOS. It attaches
SHA-256 checksum files and creates a **draft** GitHub Release for review.

## Triggering a release

- Push a version tag such as `v1.2.3`.
- Or run **Actions → Release → Run workflow** and enter an app version such as
  `alpha.0.0.1`, `a0.0.4`, or `1.2.3`. The `a`, `b`, and `rc` shorthands are
  expanded to `alpha`, `beta`, and `rc` before SemVer conversion. For example,
  `a0.0.4` becomes package version `0.0.4-alpha` and displayed version
  `alpha.0.0.4`. The workflow creates a `v`-prefixed tag and uses the normalized
  display value for the Android APK. The Android version code comes from the
  workflow run number. The manual run form includes
  switches for Android, Windows, Linux, and macOS builds; all are enabled by
  default. Disabling a platform also skips its CLI binary build (Windows,
  Linux, or macOS). At least one platform must be selected to publish a draft
  release.

Tag-triggered releases always build every platform.

Tag-triggered releases use the tag name as the Android APK's displayed
version. Cargo and Tauri package versions remain defined by their manifests.
Local Android builds default to `0.0.0-dev` unless an `appVersion` Gradle
property is supplied.
For Windows prerelease builds, the workflow adds the numeric GitHub Actions run
number as Tauri version build metadata so WiX can create the MSI. This does not
change the app's displayed release version.

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
`APPLE_CERTIFICATE` must be the base64-encoded contents of an exported
PKCS#12 (`.p12`) certificate that includes its private key. For example, on
macOS, copy the encoded value with
`base64 -i certificate.p12 | tr -d '\n' | pbcopy` and paste it into the
secret. `APPLE_CERTIFICATE_PASSWORD` must match the password used when
exporting that `.p12` file. A raw `.p12` file, a certificate without its
private key, or a mismatched password will fail during macOS bundle signing.
Windows installers are currently unsigned.

Review and publish the generated draft release only after verifying its
packages, signatures, and checksums.
