package dev.fetchx.browser

import android.Manifest
import android.os.SystemClock
import org.mozilla.geckoview.GeckoSession
import java.net.URI
import java.util.ArrayDeque

data class SitePermissionPrompt(
    val id: String,
    val origin: String,
    val permission: String,
    val detail: String? = null,
)

data class AndroidRuntimePermissionRequest(
    val id: String,
    val permissions: List<String>,
)

internal fun isSupportedAndroidRuntimePermission(permission: String): Boolean =
    permission in setOf(
        Manifest.permission.CAMERA,
        Manifest.permission.RECORD_AUDIO,
        Manifest.permission.ACCESS_COARSE_LOCATION,
        Manifest.permission.ACCESS_FINE_LOCATION,
    )

internal fun androidPermissionCategories(permissions: Array<out String>): List<String>? {
    val categories = permissions.map { permission ->
        when (permission) {
            Manifest.permission.CAMERA -> "camera"
            Manifest.permission.RECORD_AUDIO -> "microphone"
            Manifest.permission.ACCESS_COARSE_LOCATION,
            Manifest.permission.ACCESS_FINE_LOCATION,
            -> "location"
            else -> return null
        }
    }.distinct().sorted()
    return categories.takeIf { it.isNotEmpty() }
}

internal fun androidPermissionsForCategories(categories: List<String>): List<String>? {
    val permissions = categories.flatMap { category ->
        when (category) {
            "camera" -> listOf(Manifest.permission.CAMERA)
            "microphone" -> listOf(Manifest.permission.RECORD_AUDIO)
            "location" -> listOf(
                Manifest.permission.ACCESS_COARSE_LOCATION,
                Manifest.permission.ACCESS_FINE_LOCATION,
            )
            else -> return null
        }
    }
    return permissions.distinct().takeIf { it.isNotEmpty() }
}

internal fun areAndroidRuntimePermissionsGranted(
    permissions: List<String>,
    isGranted: (String) -> Boolean,
): Boolean {
    if (permissions.isEmpty() || permissions.any { !isSupportedAndroidRuntimePermission(it) }) return false

    val locationPermissions = permissions.filter {
        it == Manifest.permission.ACCESS_COARSE_LOCATION ||
            it == Manifest.permission.ACCESS_FINE_LOCATION
    }
    val otherPermissions = permissions - locationPermissions.toSet()
    return otherPermissions.all(isGranted) &&
        (locationPermissions.isEmpty() || locationPermissions.any(isGranted))
}

internal class OneShotPermissionDecisions(
    private val nowMs: () -> Long = SystemClock::elapsedRealtime,
    private val lifetimeMs: Long = 15_000L,
) {
    private data class Decision(val createdAtMs: Long, val allowed: Boolean)

    private val decisions = mutableMapOf<String, ArrayDeque<Decision>>()

    @Synchronized
    fun remember(origin: String, categories: List<String>, allowed: Boolean) {
        decisions.getOrPut(key(origin, categories), ::ArrayDeque)
            .addLast(Decision(nowMs(), allowed))
    }

    @Synchronized
    fun consume(origin: String, categories: List<String>): Boolean? {
        val now = nowMs()
        val key = key(origin, categories)
        val queue = decisions[key] ?: return null
        while (queue.isNotEmpty() && now - queue.first().createdAtMs > lifetimeMs) {
            queue.removeFirst()
        }
        val decision = queue.pollFirst()
        if (queue.isEmpty()) decisions.remove(key)
        return decision?.allowed
    }

    private fun key(origin: String, categories: List<String>): String =
        "${displayPermissionOrigin(origin)}|${categories.distinct().sorted().joinToString("|")}"
}

internal fun sitePermissionDescription(permission: Int): Pair<String, String?>? =
    when (permission) {
        GeckoSession.PermissionDelegate.PERMISSION_GEOLOCATION ->
            "location" to null
        GeckoSession.PermissionDelegate.PERMISSION_DESKTOP_NOTIFICATION ->
            "notifications" to null
        GeckoSession.PermissionDelegate.PERMISSION_PERSISTENT_STORAGE ->
            "persistent storage" to "Once granted, GeckoView does not support revoking this permission."
        GeckoSession.PermissionDelegate.PERMISSION_XR ->
            "virtual reality features" to null
        GeckoSession.PermissionDelegate.PERMISSION_AUTOPLAY_INAUDIBLE ->
            "silent media autoplay" to null
        GeckoSession.PermissionDelegate.PERMISSION_AUTOPLAY_AUDIBLE ->
            "media autoplay with sound" to null
        GeckoSession.PermissionDelegate.PERMISSION_MEDIA_KEY_SYSTEM_ACCESS ->
            "protected media playback" to null
        GeckoSession.PermissionDelegate.PERMISSION_TRACKING ->
            "permission to bypass tracking protection" to null
        GeckoSession.PermissionDelegate.PERMISSION_STORAGE_ACCESS ->
            "third-party storage access" to null
        GeckoSession.PermissionDelegate.PERMISSION_LOCAL_DEVICE_ACCESS ->
            "local device access" to null
        GeckoSession.PermissionDelegate.PERMISSION_LOCAL_NETWORK_ACCESS ->
            "local network access" to null
        else -> null
    }

internal fun shouldPromptForContentPermission(permission: Int, value: Int): Boolean =
    value == GeckoSession.PermissionDelegate.ContentPermission.VALUE_PROMPT ||
        // GeckoView 158 omits the unset geolocation value, which is decoded as zero.
        (permission == GeckoSession.PermissionDelegate.PERMISSION_GEOLOCATION && value == 0)

internal fun displayPermissionOrigin(uri: String): String {
    return try {
        val parsed = URI(uri)
        val scheme = parsed.scheme ?: return uri
        val host = parsed.host ?: return uri
        val port = parsed.port.takeIf { it >= 0 }?.let { ":$it" }.orEmpty()
        "$scheme://$host$port"
    } catch (_: Exception) {
        uri
    }
}
