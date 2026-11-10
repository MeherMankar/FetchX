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
property is supplied. Release APKs use the normalized release version as their
Android version name, and the GitHub Actions run number as their version code.
The installed app also shows its version under **Browser settings → About**.
For Windows prerelease builds, the workflow adds the numeric GitHub Actions run
number as Tauri version build metadata so WiX can create the MSI. This does not
change the app's displayed release version.

## Android signing secrets

Android release builds require a persistent signing key. Generate a keystore
once and keep a secure backup: every future update must use the same key. If
the key is lost or replaced, users will have to uninstall the app before
installing the newly signed version.

Generate a key with `keytool`:

```sh
keytool -genkeypair -v -storetype JKS -keystore fetchx-release.jks -alias fetchx \
  -keyalg RSA -keysize 4096 -validity 10000
```

Add these repository secrets under **Settings → Secrets and variables →
Actions**:

- `ANDROID_KEYSTORE_BASE64`: base64-encoded JKS/PKCS12 keystore
- `ANDROID_KEY_ALIAS`
- `ANDROID_KEY_PASSWORD`
- `ANDROID_STORE_PASSWORD`

Encode the keystore without line breaks before saving it as
`ANDROID_KEYSTORE_BASE64` (for example, `base64 -w0 fetchx-release.jks` on
Linux, or `[Convert]::ToBase64String([IO.File]::ReadAllBytes("fetchx-release.jks")) | Set-Clipboard`
in PowerShell). Never commit the keystore or its passwords. The Android
release job fails early if any of the four secrets are missing, and Gradle
signs release variants with this key; debug builds do not require it.

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
