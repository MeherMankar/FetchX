package dev.fetchx.downloads.drm

import android.util.Log
import javax.crypto.Cipher
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

private const val TAG = "DrmSegmentDecryptor"

/**
 * Decrypts Widevine CENC (AES-128-CTR) encrypted segment bytes using a known content key.
 *
 * Usage:
 * 1. Check DrmKeyStore for a known key by KID.
 * 2. If found, call decrypt() on the raw segment bytes before writing to disk.
 * 3. If not found, call WidevineManager.requestKeys() to fetch, store in DrmKeyStore, then decrypt.
 */
object DrmSegmentDecryptor {

    /**
     * Decrypts [encryptedBytes] using AES-128-CTR with [keyHex] and [ivHex].
     *
     * @param encryptedBytes  Raw encrypted segment bytes.
     * @param keyHex          Content key as 32-character hex string (16 bytes).
     * @param ivHex           Initialization vector as 32-character hex string (16 bytes).
     *                        For CENC, this is derived from the sample IV in the segment.
     *                        Pass a 32-char zero string if not available (some streams use block offset 0).
     * @return Decrypted bytes, or the original bytes if decryption fails.
     */
    fun decrypt(encryptedBytes: ByteArray, keyHex: String, ivHex: String): ByteArray {
        return try {
            val keyBytes = hexToBytes(keyHex)
            val ivBytes  = hexToBytes(ivHex)
            val cipher = Cipher.getInstance("AES/CTR/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(keyBytes, "AES"), IvParameterSpec(ivBytes))
            cipher.doFinal(encryptedBytes)
        } catch (e: Exception) {
            Log.e(TAG, "Segment decryption failed — returning original bytes", e)
            encryptedBytes
        }
    }

    /**
     * Returns the AES-CTR decrypted bytes for a segment, looking up the key from [DrmKeyStore].
     *
     * @param encryptedBytes  Raw encrypted segment bytes.
     * @param kidHex          Key ID (hex) extracted from the segment or manifest.
     * @param ivHex           Sample IV (hex). Pass zeros if unknown.
     * @return Decrypted bytes if a matching key is in [DrmKeyStore]; otherwise the original bytes.
     */
    fun decryptWithStoredKey(
        encryptedBytes: ByteArray,
        kidHex: String,
        ivHex: String = "00000000000000000000000000000000",
    ): ByteArray {
        val pair = DrmKeyStore.findByKid(kidHex)
        if (pair == null) {
            Log.w(TAG, "No key found for KID $kidHex — returning encrypted bytes")
            return encryptedBytes
        }
        return decrypt(encryptedBytes, pair.second, ivHex)
    }

    private fun hexToBytes(hex: String): ByteArray {
        require(hex.length % 2 == 0) { "Hex string must have even length" }
        return ByteArray(hex.length / 2) { i -> hex.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
    }
}
