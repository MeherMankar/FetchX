# Android Security & Privacy Audit

## 1. Executive Summary

This audit reviewed the Android application, browser module, download/storage implementation, extension framework, notification flow, and the Rust bridge surface relevant to Android security. The project already includes several good controls: the service is not exported, the app does not concatenate shell commands, filenames are sanitized, tracker-host statistics discard request paths, and extension update/optional prompts are denied by default.

The audit did not find a confirmed critical or high-severity vulnerability in the current implementation. The main findings are low-risk or design-level issues rather than direct exploitation paths. The application is therefore considered to be in a `PASS WITH LIMITATIONS` state for this phase.

## 2. Scope

Reviewed components:
- Android manifest and app entry points
- Browser runtime and tab lifecycle
- Tracker blocking and privacy controls
- Extension install, lifecycle, and permissions model
- Android storage selection and filename sanitization
- Notification behavior and background service lifecycle
- Android-to-Rust bridge inputs and app-level process setup
- Relevant Rust crates and workspace code connected to Android behavior

## 3. Architecture Reviewed

- Android app: `apps/android/app`
- Browser: `apps/android/browser`
- Downloads/storage layer: `apps/android/downloads`
- Android bridge: `apps/android/bridge`
- Shared Rust crates: `crates/*`

## 4. Threat Model

Practical actors and attack pathways considered:
- Malicious website content rendered in GeckoView
- Malicious local extension package imported by the user
- Untrusted file name or path from a remote source
- A hostile or compromised network path
- Local app or OS-level leakage through notifications or logs
- Process recreation and service lifecycle ambiguity after app restarts

The model does not include speculative bypasses or unrelated security features that are not implemented in the current architecture.

## 5. Attack Surface

Key attack surfaces observed:
- Gecko-based browser content rendering and tracker blocking
- Local extension import and WebExtension installation flow
- Download destination and filename handling
- Notification creation and user-visible metadata
- Android process/service startup lifecycle
- Rust-to-JVM bridge inputs only where they interact with Android runtime state

## 6. Security Findings

### ID-20O-001 — LOW
- Severity: LOW
- Affected component: `apps/android/app/src/main/kotlin/dev/mydownloader/app/FetchXApp.kt`, `apps/android/app/src/main/kotlin/dev/mydownloader/app/service/DownloadForegroundService.kt`
- Description: The app starts the foreground service during normal app startup regardless of whether any downloads are active. The service then immediately promotes itself to the foreground and continues observing the bridge repository even when there are no active or queued downloads.
- Attack scenario: An attacker or malicious flow that causes the app to start repeatedly can keep the process alive longer than necessary and increase the app’s background footprint. This is not a privilege escalation, but it expands the runtime and service surface.
- Impact: Increased background persistence, unnecessary foreground notifications, higher resource usage, and a broader lifecycle attack surface than required.
- Evidence: The app calls `startDownloadServiceIfNeeded()` in `onCreate()`, and the service calls `startForegroundCompat(0)` before checking for active downloads.
- Remediation: Start the foreground service only when downloads are active or when the user explicitly opts into background monitoring.
- Whether fixed: Not fixed in this audit because the existing design appears intentional and the issue is operational rather than a direct exploit.
- Validation status: Confirmed by source inspection and runtime behavior reviewed on-device.

### ID-20O-002 — LOW
- Severity: LOW
- Affected component: `apps/android/browser/src/main/kotlin/dev/mydownloader/browser/ExtensionPackageImporter.kt`, `apps/android/browser/src/main/kotlin/dev/mydownloader/browser/GeckoExtensionGateway.kt`, `apps/android/browser/src/main/kotlin/dev/mydownloader/browser/ExtensionModels.kt`
- Description: The extension import path accepts a user-selected local `.xpi` package, stages it in the app cache, and installs it without any cryptographic signature verification or trust check beyond the local picker and the GeckoView install prompt. Metadata permissions are normalized, but the validation model does not prove the package is signed or trustworthy.
- Attack scenario: A malicious extension package is manually imported by a user or delivered via a malicious local file provider. If the user is tricked into installing it, the app will accept it subject to the WebExtension install prompt.
- Impact: This is a trust and integrity problem for extension installation; the app depends on the user to make a trust decision rather than verifying the package authoritatively.
- Evidence: The install flow permits `INSTALLATION_METHOD_FROM_FILE` and performs metadata validation, but not signature or trust validation. The prompt delegate auto-denies update/optional prompts and only approves the initial install path when the extension requests no privileged permissions.
- Remediation: Validate the extension package signature against a trusted repository or locally trusted certificate metadata before enabling it, and require explicit user confirmation beyond a basic package import flow.
- Whether fixed: Not fixed in this audit because the current architecture intentionally allows local extension import and does not yet implement a signed-extension trust model.
- Validation status: Confirmed by source inspection; no exploit was demonstrated in the current environment.

### ID-20O-003 — INFORMATIONAL
- Severity: INFORMATIONAL
- Affected component: `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/notification/DownloadNotificationManager.kt`
- Description: Download notifications include file names such as the target download title. This is useful for user feedback but can expose file names on a lock screen or in an accessible notification context.
- Attack scenario: A shared device or local attacker with access to the notification shade can identify downloaded content names.
- Impact: Privacy leakage of downloaded content names, but not a secret exfiltration vector by itself.
- Evidence: `notifyProgress`, `notifyCompleted`, and related functions set `setContentTitle(item.fileName)` and similar metadata.
- Remediation: Provide an option to hide file names from lock-screen notifications or replace with generic labels in sensitive contexts.
- Whether fixed: Intentionally not changed in this phase because explicit file names are part of a valid download-status UX and the behavior is not a confirmed exploit.
- Validation status: Confirmed by source inspection.

## 7. Privacy Findings

- The tracker blocking implementation intentionally strips request URLs and query strings. It retains only normalized hostnames and bounded counts, which is a good privacy-minimizing behavior.
- The app does not log full URLs, credentials, tokens, cookies, or sensitive request data in the tracker blocking path.
- File names are surfaced in notifications, which is a user-visible privacy tradeoff rather than a direct leak via logs or network.

## 8. Android Configuration Findings

- `MainActivity` is exported for launcher use, which is correct.
- `DownloadForegroundService` is intentionally non-exported (`android:exported="false"`), which is a good boundary.
- `POST_NOTIFICATIONS` is requested and granted on the device, which is consistent with the app’s notification feature.
- No obvious accidental export of receivers/providers was identified from the manifest and app configuration reviewed.

## 9. GeckoView Findings

- The GeckoView integration follows the project’s intended design: one runtime, per-tab session, tracker blocking callback, and bounded in-memory statistics.
- No evidence of synthetic callback fabrication or custom tracker-count mutation was found.
- GeckoView native behavior is used instead of custom URL matching logic for tracker blocking.

## 10. Extension Findings

- Extension install/update/optional prompts are deliberately denied for updates and optional permissions; the app is conservative in that respect.
- Extension permission normalization rejects malformed values and converts unsupported permissions to an explicit unsupported marker.
- The main remaining concern is the absence of cryptographic trust validation for imported local packages.

## 11. Storage Findings

- `StorageManager` sanitizes file names and rejects traversal patterns such as `..` and path separators.
- The custom directory path is stored as a persisted URI permission and is only used if the permission is still valid.
- No shell command concatenation or direct unsanitized filesystem injection was found in the storage path.

## 12. Notification Findings

- Notifications use Android channels and are handled conservatively.
- No exported notification action receiver or unvalidated pending intent was identified.
- The only notable privacy issue is the display of file names, which is a UX tradeoff rather than a direct exploit.

## 13. Rust Bridge Findings

- The Rust/native boundary is not a direct source of command injection in the reviewed code paths.
- The project’s rules and command execution patterns prefer structured arguments and avoid shell concatenation.
- No direct unsafe `unwrap()` or `expect()` risks were identified in the reviewed Android bridge paths that would create a crash-prone exploit chain.

## 14. Downloader / Process Findings

- The downloader and engine code review did not reveal shell-string concatenation or unsafe executable build-up in the reviewed commands.
- The project’s process supervision architecture remains narrow and intentionally structured.
- No direct privilege-escalation path was identified in the process layer.

## 15. Dependency Findings

- The project uses GeckoView and Android platform APIs without obvious misconfigurations in the reviewed files.
- No dependency upgrade was required or justified during this audit.
- No immediate dependency security issue was identified from the reviewed configuration and project state.

## 16. Positive Security Controls Already Present

- Safe filesystem naming and traversal prevention
- Non-exported Android service
- Structured non-shell command execution patterns
- Bounded tracker statistics that discard paths and query strings
- Denial of update/optional extension permission prompts
- No raw credential or secret logging in the reviewed browser, storage, and tracker paths

## 17. Limitations / Inconclusive Tests

- This audit was source-driven and validated against the current codebase and runtime behavior available in the environment.
- Full end-to-end malicious-extension exploitation or adversarial network-path testing was not simulated in a production-like environment.
- The project is still in a foundation phase, so broader red-team testing of browser and extension behavior remains future work.

## 18. Recommended Remediation

1. Revisit the unconditional foreground service startup and only start it when downloads are active or user-authorized.
2. Add a trust or signature-validation step for imported local extension packages before the app enables them.
3. Add an option to hide file names from lock-screen notifications when the user prefers a privacy-preserving presentation.
4. Keep the current security guards while continuing targeted security review across any future browser, extension, or network features.

## 19. Validation Performed

Executed in the project environment:
- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cd apps/android; ./gradlew test --console=plain`
- `git --no-pager diff --check`

Results:
- Rust validation passed.
- Android unit test validation passed.
- Repository diff check passed with no whitespace or patch issues.

## 20. Final Risk Assessment

Phase status: PASS WITH LIMITATIONS

Risk assessment:
- Critical: 0
- High: 0
- Medium: 0
- Low: 2
- Informational: 1

The codebase currently does not show a confirmed critical security compromise, but a few low-risk design concerns remain. The application is acceptable for the current phase without introducing speculative or unrelated security mechanisms.


## 10. Remediation Status

The following Phase 20O findings were remediated in code and validated:

### 10.1 Foreground Service Remediation
- Root cause: FetchXApp.onCreate() called startDownloadServiceIfNeeded() unconditionally, even when the bridge repository had no active or queued work.
- Implementation: the app now checks BridgeInitializer.repositoryOrNull()?.hasRelevantDownloads() before starting the service. The service itself survives only so long as there is active or queued work in the authoritative repository state.
- Active states: DownloadState.Active and DownloadState.Queued keep the service alive.
- Stopping behavior: when the last active/queued item leaves the relevant lifecycle, the service calls stopForeground(...) and stopSelf(...).
- Process recreation: on app startup, the app re-evaluates the repository and starts the foreground service only when persisted state indicates relevant work is still active.
- Tests: Android unit tests for the browser and downloads modules passed, and the app compiles successfully in debug.
- Device validation: the app was validated on the physical device to confirm the service did not start without work and that it starts when required.

### 10.2 Extension Trust Remediation
- GeckoView version investigated: the project currently uses the Mozilla GeckoView dependency via the project’s Android Gradle setup and the existing browser runtime integration.
- Verification capability: the application does not implement a cryptographic signature verification flow because the current GeckoView installation path does not provide a reliable, authoritative cryptographic trust API that this application can depend on for local .xpi package verification.
- Exact model: the extension trust model is explicit and separate from permissions. Local packages are represented with ExtensionTrustState values: UNVERIFIED, TRUSTED_BY_USER, and REJECTED.
- Security model: trust does not bypass Phase 20L permissions or host permissions. The app still enforces the extension permission model and the existing lifecycle checks. We do not claim any package is cryptographically verified unless GeckoView exposes such a mechanism.
- Tests: ExtensionFrameworkTest includes coverage for local extension trust-state persistence and separation from permission handling.
- Limitations: this is a local trust-state model, not a cryptographic verification system.

### 10.3 Notification Privacy Remediation
- Implementation: DownloadNotificationPrivacySettings persists a show_download_filenames_in_notifications preference in SharedPreferences.
- Default value: the setting defaults to 	rue to preserve existing behavior unless the user disables it.
- When enabled: filenames remain visible in notification content and titles.
- When disabled: generic labels such as Download in progress, Download queued, Download completed, and Download failed are used instead of the actual filename.
- Coverage: all notification states (progress, queued, paused, completed, failed, cancelled) respect the privacy setting.
- Tests: Android notification formatting tests cover the generic-label path and retain the visible-filename path.
- Device validation: the Android device validation confirmed the setting persists across restart and is applied to new notifications.

### 10.4 Overall Assessment
The Phase 20O remediation is implemented and validated for the targeted Android behaviors without broad refactor or false security claims.
