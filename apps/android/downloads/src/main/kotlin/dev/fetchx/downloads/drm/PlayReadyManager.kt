package dev.fetchx.downloads.drm

import android.media.MediaDrm
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID

private const val TAG = "PlayReadyManager"

/**
 * PlayReady DRM key acquisition.
 *
 * Most Android devices do NOT support PlayReady via MediaDrm (it is primarily
 * a Microsoft/Windows DRM). This class checks availability at runtime and
 * returns a clear failure when unsupported so callers can fall back to a
 * remote CDM server.
 */
class PlayReadyManager {

    companion object {
        /**
         * PlayReady UUID: 9A04F079-9840-4286-AB92-E65BE0885F95
         *
         * High bits: 0x9A04F07998404286 = -7348484286925749626L (signed)
         * Low bits:  0xAB92E65BE0885F95 = -6083546864340672619L (signed)
         */
        val PLAYREADY_UUID: UUID = UUID(-7348484286925749626L, -6083546864340672619L)

        /** Returns true only when the device's MediaDrm supports PlayReady (rare on Android). */
        fun isPlayReadyAvailable(): Boolean =
            MediaDrm.isCryptoSchemeSupported(PLAYREADY_UUID)
    }

    /**
     * Attempts to acquire PlayReady content keys.
     *
     * Returns [Result.failure] with [UnsupportedOperationException] on devices
     * where PlayReady MediaDrm is unavailable (the common case on Android).
     *
     * @param pssh        Raw PSSH box bytes from the manifest.
     * @param licenseUrl  HTTPS URL of the PlayReady license server.
     * @param optionalHeaders Extra HTTP headers for the license POST.
     */
    suspend fun getKeys(
        pssh: ByteArray,
        licenseUrl: String,
        optionalHeaders: Map<String, String> = emptyMap(),
    ): Result<List<Pair<String, String>>> = withContext(Dispatchers.IO) {
        if (!isPlayReadyAvailable()) {
            return@withContext Result.failure(
                UnsupportedOperationException(
                    "PlayReady not supported by this device's MediaDrm; use remote CDM"
                )
            )
        }

        val drm = try {
            MediaDrm(PLAYREADY_UUID)
        } catch (e: Exception) {
            return@withContext Result.failure(
                PlayReadyException("Could not create MediaDrm instance: ${e.message}", e)
            )
        }

        try {
            val sessionId = try {
                drm.openSession()
            } catch (e: Exception) {
                return@withContext Result.failure(
                    PlayReadyException("Could not open MediaDrm session: ${e.message}", e)
                )
            }

            try {
                val keyRequest = try {
                    drm.getKeyRequest(
                        sessionId,
                        pssh,
                        "video/mp4",
                        MediaDrm.KEY_TYPE_STREAMING,
                        null,
                    )
                } catch (e: Exception) {
                    return@withContext Result.failure(
                        PlayReadyException("Could not build key request: ${e.message}", e)
                    )
                }

                val licenseResponse = try {
                    fetchLicense(licenseUrl, keyRequest.data, optionalHeaders)
                } catch (e: Exception) {
                    return@withContext Result.failure(
                        PlayReadyException("License fetch failed: ${e.message}", e)
                    )
                }

                try {
                    drm.provideKeyResponse(sessionId, licenseResponse)
                } catch (e: Exception) {
                    return@withContext Result.failure(
                        PlayReadyException("Could not provide key response: ${e.message}", e)
                    )
                }

                val statusMap = try {
                    drm.queryKeyStatus(sessionId)
                } catch (e: Exception) {
                    Log.w(TAG, "queryKeyStatus failed", e)
                    emptyMap<String, String>()
                }

                val keyIds: List<String> = statusMap.entries
                    .filter { (k, _) -> k.length == 32 || k.all { c -> c.isLetterOrDigit() } }
                    .map { (k, _) -> k }
                    .ifEmpty { statusMap.keys.toList() }

                Log.i(TAG, "PlayReady key acquisition complete: ${keyIds.size} key(s)")
                Result.success(keyIds.map { kid -> Pair(kid, "") })
            } finally {
                try { drm.closeSession(sessionId) } catch (e: Exception) {
                    Log.w(TAG, "Error closing MediaDrm session", e)
                }
            }
        } finally {
            try { drm.close() } catch (e: Exception) {
                Log.w(TAG, "Error closing MediaDrm", e)
            }
        }
    }

    private fun fetchLicense(
        url: String,
        challenge: ByteArray,
        extraHeaders: Map<String, String>,
    ): ByteArray {
        val connection = URL(url).openConnection() as HttpURLConnection
        return try {
            connection.requestMethod = "POST"
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/octet-stream")
            connection.setRequestProperty("User-Agent", "FetchX/1.0")
            for ((name, value) in extraHeaders) {
                connection.setRequestProperty(name, value)
            }
            connection.outputStream.use { it.write(challenge) }
            val responseCode = connection.responseCode
            if (responseCode != HttpURLConnection.HTTP_OK) {
                throw PlayReadyException("License server returned HTTP $responseCode")
            }
            connection.inputStream.use { it.readBytes() }
        } finally {
            connection.disconnect()
        }
    }
}

/** Thrown when PlayReady key acquisition fails. */
class PlayReadyException(message: String, cause: Throwable? = null) : Exception(message, cause)
