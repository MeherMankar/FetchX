package dev.mydownloader.core.download

/** Download priority — mirrors the Rust Priority enum (Phase 20B will map these via UniFFI). */
enum class DownloadPriority {
    LOW,
    NORMAL,
    HIGH,
    CRITICAL;

    companion object {
        val DEFAULT = NORMAL
    }
}
