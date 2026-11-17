package dev.fetchx.downloads.drm

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

private const val TAG = "DrmAwareDownloader"

/**
 * Downloads a media segment and decrypts it if DRM keys are available.
 *
 * Integrates with [DrmKeyStore] and [DrmSegmentDecryptor] to transparently
 * decrypt Widevine CENC-encrypted segments before returning the bytes.
 *
 * Call [downloadSegment] in place of a plain HTTP GET for each segment URL
 * when the manifest indicates PSSH/DRM protection.
 *
 * @param widevineManager  Shared [WidevineManager] instance; used to request
 *                         keys if [DrmKeyStore] does not already have them.
 */
class DrmAwareDownloader(
    private val widevineManager: WidevineManager = WidevineManager(),
) {

    /**
     * Downloads a segment from [segmentUrl] and decrypts it if DRM info is available.
     *
     * @param segmentUrl     URL of the media segment.
     * @param psshBytes      Raw PSSH box bytes from the manifest, or null if not available.
     * @param psshKey        String key used to look up [DrmKeyStore] (e.g. base64 PSSH or URL).
     * @param licenseUrl     License server URL; required when psshBytes is non-null and no
     *                       cached keys exist. May be null if keys are expected to already be
     *                       in [DrmKeyStore] (pushed from browser).
     * @param kidHex         Key ID hex string extracted from the segment/manifest, or null.
     * @param ivHex          Sample IV hex string, or null (defaults to all-zeros).
     * @param extraHeaders   Additional HTTP headers for both segment and license requests.
     * @return The segment bytes, decrypted if keys were available.
     */
    suspend fun downloadSegment(
        segmentUrl: String,
        psshBytes: ByteArray? = null,
        psshKey: String? = null,
        licenseUrl: String? = null,
        kidHex: String? = null,
        ivHex: String? = null,
        extraHeaders: Map<String, String> = emptyMap(),
    ): ByteArray = withContext(Dispatchers.IO) {
        // Step 1: fetch the raw segment bytes
        val rawBytes = fetchBytes(segmentUrl, extraHeaders)

        // Step 2: if no DRM info provided, return as-is
        if (psshBytes == null && kidHex == null && psshKey == null) {
            return@withContext rawBytes
        }

        val effectivePsshKey = psshKey ?: segmentUrl
        val effectiveIv = ivHex ?: "00000000000000000000000000000000"

        // Step 3: check DrmKeyStore first (keys may have been pushed from browser).
        // Only attempt software decryption if the stored keys carry actual key bytes (non-blank
        // second element). MediaDrm-acquired entries store an empty string as a placeholder
        // because queryKeyStatus does not expose raw key material; for those, the CDM handles
        // decryption internally and software AES-CTR must not be attempted.
        val cachedKeys = DrmKeyStore.get(effectivePsshKey)
        if (cachedKeys != null && cachedKeys.isNotEmpty()) {
            val kid = kidHex ?: cachedKeys.first().first
            val entry = DrmKeyStore.findByKid(kid) ?: cachedKeys.firstOrNull { it.first == kid }
            if (entry != null && entry.second.isNotBlank()) {
                return@withContext DrmSegmentDecryptor.decrypt(rawBytes, entry.second, effectiveIv)
            }
            // Keys present but no raw key bytes — CDM handles decryption; return raw bytes.
            Log.d(TAG, "Cached keys for $effectivePsshKey have no key bytes (MediaDrm path) — raw bytes for CDM")
            return@withContext rawBytes
        }

        // Step 4: if a KID is known, search the whole store before going to the network.
        if (kidHex != null) {
            val byKid = DrmKeyStore.findByKid(kidHex)
            if (byKid != null) {
                if (byKid.second.isNotBlank()) {
                    return@withContext DrmSegmentDecryptor.decrypt(rawBytes, byKid.second, effectiveIv)
                }
                // Found by KID but key bytes are placeholder — CDM path; return raw bytes.
                Log.d(TAG, "KID $kidHex found but key bytes empty (MediaDrm path) — raw bytes for CDM")
                return@withContext rawBytes
            }
        }

        // Step 5: request keys from license server if psshBytes are available.
        // Keys returned by WidevineManager carry empty key-byte placeholders (the CDM does not
        // expose raw key material via queryKeyStatus). Store them so future calls can detect the
        // MediaDrm path, then return the raw bytes for CDM-internal decryption.
        if (psshBytes != null && licenseUrl != null) {
            try {
                val keys = widevineManager.requestKeys(psshBytes, licenseUrl, extraHeaders)
                DrmKeyStore.put(effectivePsshKey, keys)
                // Check whether any returned key carries actual bytes (e.g. from a .wvd path).
                val usableKey = keys.firstOrNull { it.second.isNotBlank() }
                if (usableKey != null) {
                    val kid = kidHex ?: usableKey.first
                    return@withContext DrmSegmentDecryptor.decryptWithStoredKey(rawBytes, kid, effectiveIv)
                }
                // All keys are MediaDrm placeholders — return raw bytes for CDM.
                Log.d(TAG, "Keys acquired via MediaDrm for $segmentUrl — returning raw bytes for CDM")
                return@withContext rawBytes
            } catch (e: WidevineException) {
                Log.e(TAG, "Key request failed for $segmentUrl: ${e.message}", e)
            }
        }

        // Step 6: no keys available — return raw bytes (CDM handles decryption internally)
        Log.d(TAG, "No DRM keys available for $segmentUrl — returning raw bytes for CDM")
        rawBytes
    }

    private fun fetchBytes(url: String, extraHeaders: Map<String, String>): ByteArray {
        val connection = URL(url).openConnection() as HttpURLConnection
        return try {
            connection.requestMethod = "GET"
            connection.setRequestProperty("User-Agent", "FetchX/1.0")
            for ((name, value) in extraHeaders) {
                connection.setRequestProperty(name, value)
            }
            val code = connection.responseCode
            if (code != HttpURLConnection.HTTP_OK) {
                throw IllegalStateException("HTTP $code fetching segment $url")
            }
            connection.inputStream.use { it.readBytes() }
        } finally {
            connection.disconnect()
        }
    }
}
