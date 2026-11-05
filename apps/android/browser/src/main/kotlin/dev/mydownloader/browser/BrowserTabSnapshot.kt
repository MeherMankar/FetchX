package dev.fetchx.browser

import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.util.Base64

internal data class BrowserTabSnapshot(
    val generation: Long,
    val tabs: List<TabState>,
    val activeTabId: String,
)

internal object BrowserTabSnapshotCodec {
    private const val MAGIC = "FETCHX_BROWSER_TABS"
    private const val FORMAT_VERSION = "1"

    fun encode(snapshot: BrowserTabSnapshot): ByteArray {
        require(snapshot.generation > 0) { "Snapshot generation must be positive" }
        require(snapshot.tabs.isNotEmpty()) { "Snapshot must contain at least one tab" }
        require(snapshot.tabs.all { it.id.isNotBlank() }) { "Tab IDs must not be blank" }
        require(snapshot.tabs.map { it.id }.distinct().size == snapshot.tabs.size) {
            "Snapshot tab IDs must be unique"
        }
        require(snapshot.tabs.any { it.id == snapshot.activeTabId }) {
            "Active tab must belong to the snapshot"
        }

        val payload = listOf(
            FORMAT_VERSION,
            snapshot.generation.toString(),
            snapshot.tabs.size.toString(),
            encodeField(snapshot.activeTabId),
            BrowserDataCodec.encodeTabs(snapshot.tabs),
        ).joinToString("\n")
        val payloadBytes = payload.toByteArray(StandardCharsets.UTF_8)
        val envelope = listOf(
            MAGIC,
            Base64.getUrlEncoder().withoutPadding().encodeToString(payloadBytes),
            sha256(payloadBytes),
        ).joinToString("\n")
        return envelope.toByteArray(StandardCharsets.UTF_8)
    }

    fun decode(encoded: ByteArray): BrowserTabSnapshot? {
        val envelope = String(encoded, StandardCharsets.UTF_8)
        val envelopeFields = envelope.split('\n')
        if (envelopeFields.size != 3 || envelopeFields[0] != MAGIC) return null
        val payloadBytes = try {
            Base64.getUrlDecoder().decode(envelopeFields[1])
        } catch (_: IllegalArgumentException) {
            return null
        }
        if (!MessageDigest.isEqual(
                sha256(payloadBytes).toByteArray(StandardCharsets.US_ASCII),
                envelopeFields[2].toByteArray(StandardCharsets.US_ASCII),
            )
        ) {
            return null
        }

        val payload = String(payloadBytes, StandardCharsets.UTF_8).split('\n', limit = 5)
        if (payload.size != 5 || payload[0] != FORMAT_VERSION) return null
        val generation = payload[1].toLongOrNull()?.takeIf { it > 0 } ?: return null
        val expectedTabCount = payload[2].toIntOrNull()?.takeIf { it > 0 } ?: return null
        val activeTabId = decodeField(payload[3]) ?: return null
        val tabs = BrowserDataCodec.decodeTabs(payload[4])
        if (tabs.size != expectedTabCount) return null
        if (tabs.any { it.id.isBlank() } || tabs.map { it.id }.distinct().size != tabs.size) return null
        if (tabs.none { it.id == activeTabId }) return null

        return BrowserTabSnapshot(generation, tabs, activeTabId)
    }

    private fun encodeField(value: String): String =
        Base64.getUrlEncoder().withoutPadding()
            .encodeToString(value.toByteArray(StandardCharsets.UTF_8))

    private fun decodeField(value: String): String? =
        try {
            String(Base64.getUrlDecoder().decode(value), StandardCharsets.UTF_8)
        } catch (_: IllegalArgumentException) {
            null
        }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256")
            .digest(bytes)
            .joinToString("") { "%02x".format(it) }
}
