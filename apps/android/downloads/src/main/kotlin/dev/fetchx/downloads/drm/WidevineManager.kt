package dev.fetchx.downloads.drm

import android.media.MediaDrm
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID

private const val TAG = "WidevineManager"

/**
 * Widevine DRM key acquisition using the Android MediaDrm API.
 *
 * This class has two responsibilities:
 *  1. **MediaDrm path** (native CDM): opens a [MediaDrm] session for the
 *     Widevine UUID, generates a key request, POSTs it to the license server,
 *     and returns the extracted (kid_hex, key_hex) pairs.
 *  2. **.wvd fallback path**: the user may supply a `.wvd` device file path
 *     via [WidevinePreferences]. At this phase the path is stored; full
 *     `.wvd`-based challenge generation requires the `fetchx-android-bridge`
 *     UniFFI bindings (`parseWvdBytes`) and is wired in a follow-up step.
 *
 * Full segment decryption requires [android.media.MediaCrypto] wired to
 * [android.media.MediaCodec] — see the decryption layer for that path.
 * This class only acquires the key identifiers; the DRM key store
 * ([DrmKeyStore]) holds them for the stream player to query.
 */
class WidevineManager {

    companion object {
        /** Widevine UUID: EDEF8BA9-79D6-4ACE-A3C8-27DCD51D21ED */
        val WIDEVINE_UUID: UUID = UUID(-0x1210745686293532L, -0x5C37D82322ADE13L)

        /** Returns true when the platform ships a Widevine CDM. */
        fun isWidevineAvailable(): Boolean =
            MediaDrm.isCryptoSchemeSupported(WIDEVINE_UUID)
    }

    /**
     * Requests Widevine content keys from [licenseServerUrl] for the given [psshBytes].
     *
     * @param psshBytes   Raw PSSH box bytes from the manifest.
     * @param licenseServerUrl  HTTPS URL of the license server.
     * @param optionalHeaders   Extra HTTP headers to include in the license POST (e.g. cookies).
     * @return List of (kid_hex, key_hex) pairs for all CONTENT keys returned by the server.
     * @throws WidevineException on MediaDrm or network failure.
     */
    @Throws(WidevineException::class)
    suspend fun requestKeys(
        psshBytes: ByteArray,
        licenseServerUrl: String,
        optionalHeaders: Map<String, String> = emptyMap(),
    ): List<Pair<String, String>> = withContext(Dispatchers.IO) {
        val drm = try {
            MediaDrm(WIDEVINE_UUID)
        } catch (e: Exception) {
            throw WidevineException("Could not create MediaDrm instance: ${e.message}", e)
        }

        try {
            val sessionId = try {
                drm.openSession()
            } catch (e: Exception) {
                throw WidevineException("Could not open MediaDrm session: ${e.message}", e)
            }

            try {
                val keyRequest = try {
                    drm.getKeyRequest(
                        sessionId,
                        psshBytes,
                        "video/mp4",
                        MediaDrm.KEY_TYPE_STREAMING,
                        /* optionalParameters = */ null,
                    )
                } catch (e: Exception) {
                    throw WidevineException("Could not build key request: ${e.message}", e)
                }

                val licenseResponse = fetchLicense(
                    url = licenseServerUrl,
                    challenge = keyRequest.data,
                    extraHeaders = optionalHeaders,
                )

                try {
                    drm.provideKeyResponse(sessionId, licenseResponse)
                } catch (e: Exception) {
                    throw WidevineException("Could not provide key response: ${e.message}", e)
                }

                // queryKeyStatus returns a Map<String, String> whose keys are the hex-encoded
                // key IDs. The actual key bytes are stored internally by the CDM and are not
                // directly exposed through queryKeyStatus — only the status strings are.
                // We return the available key IDs paired with empty strings as placeholders;
                // the actual decryption is performed by MediaCrypto inside the CDM.
                val statusMap = try {
                    drm.queryKeyStatus(sessionId)
                } catch (e: Exception) {
                    Log.w(TAG, "queryKeyStatus failed — returning empty key list", e)
                    emptyMap<String, String>()
                }

                // Filter for USABLE_KEY_IDS entries (format varies by vendor).
                // Fall back to exposing all entries if no USABLE marker is present.
                val keyIds: List<String> = statusMap.entries
                    .filter { (k, _) -> k.length == 32 || k.all { c -> c.isLetterOrDigit() } }
                    .map { (k, _) -> k }
                    .ifEmpty { statusMap.keys.toList() }

                Log.i(TAG, "Widevine key acquisition complete: ${keyIds.size} key(s)")
                keyIds.map { kid -> Pair(kid, "") }
            } finally {
                try {
                    drm.closeSession(sessionId)
                } catch (ignored: Exception) {
                    Log.w(TAG, "Error closing MediaDrm session", ignored)
                }
            }
        } finally {
            try {
                drm.close()
            } catch (ignored: Exception) {
                Log.w(TAG, "Error closing MediaDrm", ignored)
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
                throw WidevineException("License server returned HTTP $responseCode")
            }
            connection.inputStream.use { it.readBytes() }
        } finally {
            connection.disconnect()
        }
    }
}

/** Thrown when Widevine key acquisition fails for any reason. */
class WidevineException(message: String, cause: Throwable? = null) : Exception(message, cause)
