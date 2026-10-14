# Android Storage and Notifications

Phase 20N implementation reference.

---

## 1. Storage Architecture

FetchX uses a two-tier storage model for downloads:

- **Default tier**: app-specific external directory (`Environment.DIRECTORY_DOWNLOADS` inside the app's external files dir). No dangerous permissions are required — this path is owned by the app and is automatically deleted when the app is uninstalled.
- **Custom tier**: a user-selected directory via the Storage Access Framework (SAF). The user grants a URI permission that is persisted across restarts via `takePersistableUriPermission`.

The `StorageManager` class (`:downloads` module, `dev.fetchx.downloads.storage`) manages both tiers. It reads the effective directory at runtime and falls back to default automatically if a previously granted SAF permission is later revoked.

---

## 2. Default Download Directory

The default directory is resolved as:

```kotlin
context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
    ?: context.filesDir.resolve("downloads")
```

This path is:
- App-specific: other apps cannot read it without the `READ_EXTERNAL_STORAGE` permission.
- Automatically cleaned up on uninstall.
- Not subject to scoped-storage restrictions introduced in Android 10+.
- Created on demand via `dir.mkdirs()`.

The typical resolved path is:
```
/storage/emulated/0/Android/data/dev.fetchx.app/files/Downloads/
```

---

## 3. User Directory Selection (SAF)

To let a user choose a custom download location:

1. Launch an `Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)` and receive the result URI.
2. Call `StorageManager.setCustomDirectory(uri)` with the result.
3. The manager calls `takePersistableUriPermission` to retain read+write access across restarts.
4. The selected URI is persisted in `SharedPreferences` (`fetchx_storage`).

To revert:

```kotlin
storageManager.clearCustomDirectory()
```

This releases the persisted URI permission and removes the pref.

---

## 4. Permission Model

FetchX deliberately uses NO dangerous storage permissions:

| Permission | Status |
|---|---|
| `MANAGE_EXTERNAL_STORAGE` | NOT declared — never request |
| `READ_EXTERNAL_STORAGE` | NOT declared |
| `WRITE_EXTERNAL_STORAGE` | NOT declared |
| SAF URI permission | Granted dynamically by user; persisted via `takePersistableUriPermission` |

This design works with Android's scoped storage (Android 10+) by construction. The app never accesses storage outside its own app-specific directory (default) or a SAF-granted tree (custom).

---

## 5. Filename Sanitization

All filenames received from the network are sanitized before use via `StorageManager.sanitizeFileName(raw)`:

| Transformation | Reason |
|---|---|
| Replace `/` and `\` with `_` | Prevent directory traversal via separators |
| Replace `..` with `_` | Prevent path traversal sequences |
| Replace null bytes with `_` | Prevent null-byte injection |
| Strip leading `.` characters | Prevent hidden files on Unix |
| Replace `< > : " | ? *` with `_` | Windows reserved characters |
| Truncate to 200 characters | Prevent filesystem limits |
| Fall back to `"download"` if blank | Always return a valid name |

Remote-supplied filenames are **never trusted directly**. The sanitized name is then passed through `uniqueFileName(dir, name)` to produce a non-colliding result (appending ` (1)`, ` (2)`, etc. if needed).

---

## 6. Notification Channels

Three Android notification channels are created by `DownloadNotificationManager` (`:downloads` module, `dev.fetchx.downloads.notification`):

| Channel ID | Importance | Purpose |
|---|---|---|
| `fetchx_downloads_active` | LOW (silent) | Ongoing download progress |
| `fetchx_downloads_complete` | DEFAULT | Completion alerts |
| `fetchx_downloads_failed` | DEFAULT | Failure alerts |

Channels are created in `init` — safe to call multiple times (Android is idempotent for existing channels).

**Per-download notification IDs** are derived via `downloadId.hashCode()` — stable for the same download across service restarts.

**Foreground summary notification** uses ID `1_000_000` and channel `fetchx_downloads_active`. This is the notification required to keep the foreground service alive.

**Privacy**: only `item.fileName` is shown in notifications. URLs, destination paths, and any credentials are never included.

---

## 7. Background Download Architecture

```
FetchXApp.onCreate()
    └── startDownloadServiceIfNeeded()
            └── DownloadForegroundService (foreground service)
                    └── BridgeInitializer.repositoryOrNull()
                            └── observeDownloads(): Flow<List<DownloadItem>>
                                    └── DownloadNotificationManager
                                            ├── notifyProgress()
                                            ├── notifyQueued()
                                            ├── notifyPaused()
                                            ├── notifyCompleted()
                                            ├── notifyFailed()
                                            └── notifyCancelled()
```

The service **owns no download logic**. All download operations remain in the Rust core (QueueManager/Scheduler) accessed via the existing UniFFI bridge. The service only:

1. Keeps the process alive as a foreground service.
2. Observes `DownloadRepository.observeDownloads()` and dispatches notifications.
3. Stops itself when no active or queued downloads remain.

---

## 8. Foreground Service Decision

`DownloadForegroundService` is placed in the `:app` module (`dev.fetchx.app.service`) because:

- It requires a `Context` to start, which is owned by `:app`.
- It depends on `BridgeInitializer` from `:bridge` (already a direct dep of `:app`).
- It imports `DownloadNotificationManager` from `:downloads` (already a direct dep of `:app`).
- Placing it in `:downloads` would create a circular dependency (`:downloads` → `:app`).

**Service type**: `FOREGROUND_SERVICE_TYPE_DATA_SYNC`, matching the `FOREGROUND_SERVICE_DATA_SYNC` permission declared in the manifest from Phase 20A.

**Android 14+ compliance**: `startForeground()` is called immediately in `onStartCommand()`, before any other work, satisfying the Android 14 requirement that foreground services must call `startForeground` within `onStartCommand`.

**`stopForeground` API**:
- API 33+ (Android 13 Tiramisu): `stopForeground(STOP_FOREGROUND_DETACH)` — removes the foreground state while keeping the service alive briefly to cancel any remaining notifications.
- Below API 33: `@Suppress("DEPRECATION") stopForeground(true)` — deprecated but still functional.

---

## 9. Security Considerations

- **No `MANAGE_EXTERNAL_STORAGE`**: FetchX never requests broad storage access.
- **Filename sanitization**: all path traversal sequences and reserved characters are stripped before a filename is used to create a `File`.
- **Notification privacy**: only `item.fileName` appears in notifications. `item.url`, `item.destinationPath`, credentials, and tokens are never included.
- **SAF URI permissions**: the persisted URI is checked for `isWritePermission` on every access. Stale permissions are detected and cleared.
- **`safeNotify`**: `NotificationManagerCompat.notify` is wrapped to catch `SecurityException` when `POST_NOTIFICATIONS` has not been granted (Android 13+).
- **Service not exported**: `android:exported="false"` ensures the service cannot be started by other apps.

---

## 10. Known Limitations

1. **No download throttling for notifications**: `notifyProgress` is called on every emission from `observeDownloads()`. If the Rust core emits high-frequency updates (e.g., many times per second), the notification update rate may be higher than optimal. A debounce/throttle should be added before Phase 20O.

2. **Service started unconditionally**: `FetchXApp.startDownloadServiceIfNeeded()` starts the foreground service on every app launch, regardless of whether any downloads are pending. If the bridge is unavailable, the service calls `stopSelf` immediately — but the brief foreground service start may show a notification flash. A check for pending downloads before starting would be cleaner.

3. **No open-file action on completion**: Completed download notifications have no `PendingIntent` to open the file. This requires a `FileProvider` setup (Phase 20P or later).

4. **In-memory custom directory state**: if the process is killed and the custom URI permission is revoked by the system before the next launch, the `SharedPreferences` key still holds the old URI until the next `resolveDownloadDirectoryPath()` call detects the missing permission.

5. **Pre-existing Phase 20C UnsatisfiedLinkError**: the native bridge `.so` issue for JNA/UniFFI remains unresolved. The service correctly handles `BridgeInitializer.repositoryOrNull()` returning `null` (stops immediately).
