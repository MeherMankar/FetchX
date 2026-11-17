package dev.fetchx.downloads.drm

import java.util.concurrent.ConcurrentHashMap

/**
 * In-memory store for Widevine DRM keys keyed by PSSH data string.
 *
 * Keys are populated by [WidevineManager.requestKeys] or forwarded from the
 * browser extension via the native messaging bridge. The stream player queries
 * this store to retrieve the key pairs needed to decrypt protected segments.
 *
 * Full segment decryption using [android.media.MediaCrypto] is wired to
 * [android.media.MediaCodec] — see the decryption layer for that path.
 * This store only holds the (kid_hex, key_hex) pairs; the CDM performs the
 * actual per-sample decryption.
 */
object DrmKeyStore {

    private val store = ConcurrentHashMap<String, List<Pair<String, String>>>()

    /**
     * Store the given [keys] under [psshKey].
     *
     * @param psshKey  Identifier for the content (usually the base64-encoded PSSH or its URL).
     * @param keys     List of (kid_hex, key_hex) pairs returned by [WidevineManager.requestKeys].
     */
    fun put(psshKey: String, keys: List<Pair<String, String>>) {
        store[psshKey] = keys
    }

    /**
     * Retrieve the DRM keys previously stored under [psshKey], or null if none are known.
     */
    fun get(psshKey: String): List<Pair<String, String>>? = store[psshKey]

    /**
     * Find a key by its key ID (hex string, case-insensitive) across all stored entries.
     *
     * @return The (kid_hex, key_hex) pair if found, or null.
     */
    fun findByKid(kidHex: String): Pair<String, String>? {
        for (keys in store.values) {
            val match = keys.firstOrNull { (kid, _) ->
                kid.equals(kidHex, ignoreCase = true)
            }
            if (match != null) return match
        }
        return null
    }

    /** Remove all stored DRM keys. */
    fun clear() {
        store.clear()
    }
}
