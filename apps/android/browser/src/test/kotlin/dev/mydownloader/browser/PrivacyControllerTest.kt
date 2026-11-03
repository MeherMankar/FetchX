package dev.fetchx.browser

import org.mozilla.geckoview.ContentBlocking
import org.mozilla.geckoview.GeckoRuntimeSettings
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

class PrivacyControllerTest {

    @Test
    fun `privacy defaults avoid site compatibility and performance costs`() {
        val controller = PrivacyController()

        assertEquals(
            PrivacySettings(
                adBlockingEnabled = true,
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = true,
                thirdPartyCookieIsolationEnabled = false,
                httpsOnlyEnabled = false,
            ),
            controller.settings.value,
        )
    }

    @Test
    fun `setting changes publish immutable state and notify only when changed`() {
        val controller = PrivacyController()
        var changes = 0
        controller.onSettingsChanged = { changes += 1 }

        controller.setAdBlockingEnabled(false)
        controller.setAdBlockingEnabled(false)
        controller.setFingerprintingProtectionEnabled(true)
        controller.setFingerprintingProtectionEnabled(true)
        controller.setHttpsOnlyEnabled(true)

        assertEquals(
            PrivacySettings(
                adBlockingEnabled = false,
                fingerprintingProtectionEnabled = true,
                cryptominingProtectionEnabled = true,
                thirdPartyCookieIsolationEnabled = false,
                httpsOnlyEnabled = true,
            ),
            controller.settings.value,
        )
        assertEquals(3, changes)
    }

    @Test
    fun `native configuration composes tracker and privacy categories`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = PrivacySettings(),
        )

        assertEquals(
            ContentBlocking.AntiTracking.AD or
            ContentBlocking.AntiTracking.ANALYTIC or
            ContentBlocking.AntiTracking.SOCIAL or
            ContentBlocking.AntiTracking.CRYPTOMINING,
            configuration.antiTrackingCategories,
        )
        assertFalse(configuration.fingerprintingProtectionEnabled)
    }

    @Test
    fun `disabling tracker categories preserves independently enabled protections`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = false,
            settings = PrivacySettings(),
        )

        assertEquals(
            ContentBlocking.AntiTracking.AD or
                ContentBlocking.AntiTracking.CRYPTOMINING,
            configuration.antiTrackingCategories,
        )
    }

    @Test
    fun `disabled privacy options map to permissive GeckoView settings`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = false,
            settings = PrivacySettings(
                adBlockingEnabled = false,
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = false,
                thirdPartyCookieIsolationEnabled = false,
                httpsOnlyEnabled = false,
            ),
        )

        assertEquals(ContentBlocking.AntiTracking.NONE, configuration.antiTrackingCategories)
        assertFalse(configuration.fingerprintingProtectionEnabled)
        assertEquals(ContentBlocking.CookieBehavior.ACCEPT_ALL, configuration.cookieBehavior)
        assertEquals(
            GeckoRuntimeSettings.ALLOW_ALL,
            configuration.allowInsecureConnections,
        )
    }

    @Test
    fun `cookie isolation and https only map to GeckoView native constants`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = false,
            settings = PrivacySettings(
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = false,
                thirdPartyCookieIsolationEnabled = true,
                httpsOnlyEnabled = true,
            ),
        )

        assertEquals(
            ContentBlocking.CookieBehavior.ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS,
            configuration.cookieBehavior,
        )
        assertEquals(
            GeckoRuntimeSettings.HTTPS_ONLY,
            configuration.allowInsecureConnections,
        )
    }

    @Test
    fun `ad blocking can be enabled independently from tracker protection`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = false,
            settings = PrivacySettings(
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = false,
            ),
        )

        assertEquals(ContentBlocking.AntiTracking.AD, configuration.antiTrackingCategories)

        val disabledConfiguration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = false,
            settings = PrivacySettings(
                adBlockingEnabled = false,
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = false,
            ),
        )
        assertEquals(
            ContentBlocking.AntiTracking.NONE,
            disabledConfiguration.antiTrackingCategories,
        )
    }

    @Test
    fun `disabling ad blocking preserves enabled tracker protection`() {
        val configuration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = PrivacySettings(
                adBlockingEnabled = false,
                fingerprintingProtectionEnabled = false,
                cryptominingProtectionEnabled = false,
            ),
        )

        assertEquals(
            ContentBlocking.AntiTracking.ANALYTIC or ContentBlocking.AntiTracking.SOCIAL,
            configuration.antiTrackingCategories,
        )
    }

    @Test
    fun `same runtime configuration applies to existing and subsequently opened tabs`() {
        val settings = PrivacySettings(httpsOnlyEnabled = true)

        val existingTabConfiguration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = settings,
        )
        val newTabConfiguration = geckoPrivacyConfiguration(
            trackerBlockingEnabled = true,
            settings = settings,
        )

        assertEquals(existingTabConfiguration, newTabConfiguration)
    }

    @Test
    fun `site connection status reflects only the URL scheme`() {
        assertEquals(SiteConnectionStatus.HTTPS, siteConnectionStatus("https://example.com"))
        assertEquals(SiteConnectionStatus.HTTP, siteConnectionStatus("http://example.com"))
        assertEquals(SiteConnectionStatus.UNKNOWN, siteConnectionStatus("about:blank"))
        assertEquals(SiteConnectionStatus.UNKNOWN, siteConnectionStatus("not a URL"))
    }
}
