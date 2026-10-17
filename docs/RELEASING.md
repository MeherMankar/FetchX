# Creating FetchX releases

The GitHub Actions release workflow builds Android, Windows, Linux, and macOS
desktop packages, plus CLI binaries for Windows, Linux, and macOS. It attaches
SHA-256 checksum files and creates a **draft** GitHub Release for review.

## Triggering a release

- Push a version tag such as `v1.2.3`.
- Or run **Actions → Release → Run workflow** and enter a version such as
  `1.2.3`. The workflow creates the corresponding `v1.2.3` tag at the selected
  commit and creates a draft release.

The workflow does not update version fields in the project manifests. Update
the Android, Cargo, and Tauri version values in the source before building a
release when they need to change.

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
