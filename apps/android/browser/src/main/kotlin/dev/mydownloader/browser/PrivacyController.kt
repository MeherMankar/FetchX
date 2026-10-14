package dev.fetchx.browser

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.mozilla.geckoview.ContentBlocking
import org.mozilla.geckoview.GeckoRuntimeSettings

data class PrivacySettings(
    val adBlockingEnabled: Boolean = true,
    val fingerprintingProtectionEnabled: Boolean = true,
    val cryptominingProtectionEnabled: Boolean = true,
    val thirdPartyCookieIsolationEnabled: Boolean = true,
    val httpsOnlyEnabled: Boolean = false,
)

class PrivacyController(initialSettings: PrivacySettings = PrivacySettings()) {
    private val lock = Any()
    private val _settings = MutableStateFlow(initialSettings)

    val settings: StateFlow<PrivacySettings> = _settings.asStateFlow()

    @Volatile
    var onSettingsChanged: (() -> Unit)? = null

    fun setAdBlockingEnabled(enabled: Boolean) =
        update { it.copy(adBlockingEnabled = enabled) }

    fun setFingerprintingProtectionEnabled(enabled: Boolean) =
        update { it.copy(fingerprintingProtectionEnabled = enabled) }

    fun setCryptominingProtectionEnabled(enabled: Boolean) =
        update { it.copy(cryptominingProtectionEnabled = enabled) }

    fun setThirdPartyCookieIsolationEnabled(enabled: Boolean) =
        update { it.copy(thirdPartyCookieIsolationEnabled = enabled) }

    fun setHttpsOnlyEnabled(enabled: Boolean) =
        update { it.copy(httpsOnlyEnabled = enabled) }

    private fun update(transform: (PrivacySettings) -> PrivacySettings) {
        val changed = synchronized(lock) {
            val updated = transform(_settings.value)
            if (updated == _settings.value) {
                false
            } else {
                _settings.value = updated
                true
            }
        }
        if (changed) onSettingsChanged?.invoke()
    }
}

internal data class GeckoPrivacyConfiguration(
    val antiTrackingCategories: Int,
    val fingerprintingProtectionEnabled: Boolean,
    val cookieBehavior: Int,
    val allowInsecureConnections: Int,
)

internal fun geckoPrivacyConfiguration(
    trackerBlockingEnabled: Boolean,
    settings: PrivacySettings,
): GeckoPrivacyConfiguration {
    var categories = trackerAntiTrackingCategories(trackerBlockingEnabled)
    categories = if (settings.adBlockingEnabled) {
        categories or ContentBlocking.AntiTracking.AD
    } else {
        categories and ContentBlocking.AntiTracking.AD.inv()
    }
    if (settings.fingerprintingProtectionEnabled) {
        categories = categories or ContentBlocking.AntiTracking.FINGERPRINTING
    }
    if (settings.cryptominingProtectionEnabled) {
        categories = categories or ContentBlocking.AntiTracking.CRYPTOMINING
    }

    return GeckoPrivacyConfiguration(
        antiTrackingCategories = categories,
        fingerprintingProtectionEnabled = settings.fingerprintingProtectionEnabled,
        cookieBehavior = if (settings.thirdPartyCookieIsolationEnabled) {
            ContentBlocking.CookieBehavior.ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS
        } else {
            ContentBlocking.CookieBehavior.ACCEPT_ALL
        },
        allowInsecureConnections = if (settings.httpsOnlyEnabled) {
            GeckoRuntimeSettings.HTTPS_ONLY
        } else {
            GeckoRuntimeSettings.ALLOW_ALL
        },
    )
}
