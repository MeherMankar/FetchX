package dev.fetchx.downloads.notification

fun formatDownloadSpeed(bytesPerSec: Long): String = when {
    bytesPerSec >= 1_048_576 -> String.format("%.1f MB/s", bytesPerSec / 1_048_576.0)
    bytesPerSec >= 1_024     -> String.format("%.0f KB/s", bytesPerSec / 1_024.0)
    else                     -> "$bytesPerSec B/s"
}

fun formatEta(seconds: Long): String = when {
    seconds < 60    -> "${seconds}s"
    seconds < 3_600 -> "${seconds / 60}m"
    else            -> "${seconds / 3_600}h ${(seconds % 3_600) / 60}m"
}

fun formatBytes(bytes: Long): String = when {
    bytes >= 1_073_741_824 -> String.format("%.2f GB", bytes / 1_073_741_824.0)
    bytes >= 1_048_576     -> String.format("%.1f MB", bytes / 1_048_576.0)
    bytes >= 1_024         -> String.format("%.0f KB", bytes / 1_024.0)
    else                   -> "$bytes B"
}
