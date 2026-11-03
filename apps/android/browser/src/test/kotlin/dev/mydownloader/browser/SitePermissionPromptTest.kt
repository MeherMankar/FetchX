package dev.fetchx.browser

import android.Manifest
import org.junit.Test
import org.mozilla.geckoview.GeckoSession
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class SitePermissionPromptTest {

    @Test
    fun `supported GeckoView content permissions have user-facing descriptions`() {
        assertEquals(
            "location",
            sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_GEOLOCATION)?.first,
        )
        assertEquals(
            "notifications",
            sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_DESKTOP_NOTIFICATION)?.first,
        )
        assertEquals(
            "persistent storage",
            sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_PERSISTENT_STORAGE)?.first,
        )
        assertNotNull(sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_AUTOPLAY_AUDIBLE))
        assertNotNull(sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_STORAGE_ACCESS))
        assertNotNull(sitePermissionDescription(GeckoSession.PermissionDelegate.PERMISSION_LOCAL_NETWORK_ACCESS))
        assertNull(sitePermissionDescription(Int.MIN_VALUE))
    }

    @Test
    fun `only prompt and unset geolocation values enter site consent flow`() {
        val prompt = GeckoSession.PermissionDelegate.ContentPermission.VALUE_PROMPT
        val allow = GeckoSession.PermissionDelegate.ContentPermission.VALUE_ALLOW
        val deny = GeckoSession.PermissionDelegate.ContentPermission.VALUE_DENY
        val geolocation = GeckoSession.PermissionDelegate.PERMISSION_GEOLOCATION
        val notifications = GeckoSession.PermissionDelegate.PERMISSION_DESKTOP_NOTIFICATION

        assertTrue(shouldPromptForContentPermission(geolocation, prompt))
        assertTrue(shouldPromptForContentPermission(geolocation, 0))
        assertFalse(shouldPromptForContentPermission(notifications, 0))
        assertFalse(shouldPromptForContentPermission(geolocation, allow))
        assertFalse(shouldPromptForContentPermission(geolocation, deny))
        assertFalse(shouldPromptForContentPermission(geolocation, Int.MIN_VALUE))
    }

    @Test
    fun `origin display omits path and query while preserving a non-default port`() {
        assertEquals(
            "https://example.com:8443",
            displayPermissionOrigin("https://example.com:8443/camera?token=private"),
        )
        assertEquals("opaque value", displayPermissionOrigin("opaque value"))
    }

    @Test
    fun `only declared Android runtime permissions are accepted`() {
        assertTrue(isSupportedAndroidRuntimePermission(Manifest.permission.CAMERA))
        assertTrue(isSupportedAndroidRuntimePermission(Manifest.permission.RECORD_AUDIO))
        assertTrue(isSupportedAndroidRuntimePermission(Manifest.permission.ACCESS_COARSE_LOCATION))
        assertTrue(isSupportedAndroidRuntimePermission(Manifest.permission.ACCESS_FINE_LOCATION))
        assertFalse(isSupportedAndroidRuntimePermission(Manifest.permission.POST_NOTIFICATIONS))
        assertFalse(isSupportedAndroidRuntimePermission("android.permission.UNKNOWN"))
    }

    @Test
    fun `Android permissions map only to supported site categories`() {
        assertEquals(
            listOf("camera", "location"),
            androidPermissionCategories(
                arrayOf(
                    Manifest.permission.CAMERA,
                    Manifest.permission.ACCESS_COARSE_LOCATION,
                    Manifest.permission.ACCESS_FINE_LOCATION,
                ),
            ),
        )
        assertEquals(
            listOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION),
            androidPermissionsForCategories(listOf("location")),
        )
        assertNull(androidPermissionCategories(arrayOf(Manifest.permission.POST_NOTIFICATIONS)))
        assertNull(androidPermissionsForCategories(listOf("unknown")))
    }

    @Test
    fun `approximate location permission is sufficient but other permissions remain required`() {
        val locationPermissions = listOf(
            Manifest.permission.ACCESS_COARSE_LOCATION,
            Manifest.permission.ACCESS_FINE_LOCATION,
        )
        val cameraAndLocation = locationPermissions + Manifest.permission.CAMERA

        assertTrue(
            areAndroidRuntimePermissionsGranted(locationPermissions) {
                it == Manifest.permission.ACCESS_COARSE_LOCATION
            },
        )
        assertTrue(
            areAndroidRuntimePermissionsGranted(locationPermissions) {
                it == Manifest.permission.ACCESS_FINE_LOCATION
            },
        )
        assertFalse(areAndroidRuntimePermissionsGranted(locationPermissions) { false })
        assertFalse(
            areAndroidRuntimePermissionsGranted(cameraAndLocation) {
                it == Manifest.permission.ACCESS_COARSE_LOCATION
            },
        )
        assertFalse(areAndroidRuntimePermissionsGranted(emptyList()) { true })
    }

    @Test
    fun `one-shot Android permission decision is origin scoped and expires`() {
        var now = 100L
        val decisions = OneShotPermissionDecisions(nowMs = { now }, lifetimeMs = 10L)
        val categories = listOf("camera")

        decisions.remember("https://first.example/page", categories, allowed = false)

        assertNull(decisions.consume("https://second.example/page", categories))
        assertEquals(false, decisions.consume("https://first.example/other", categories))
        assertNull(decisions.consume("https://first.example/page", categories))

        decisions.remember("https://first.example", categories, allowed = true)
        now += 11L

        assertNull(decisions.consume("https://first.example", categories))
    }

    @Test
    fun `site permission prompts are resolved in request order`() {
        val viewModel = BrowserViewModel()
        val decisions = mutableListOf<Boolean>()

        viewModel.enqueueSitePermission("https://first.example", "camera", null, decisions::add)
        viewModel.enqueueSitePermission("https://second.example", "microphone", null, decisions::add)

        val firstPrompt = assertNotNull(viewModel.uiState.value.sitePermissionPrompt)
        assertEquals("https://first.example", firstPrompt.origin)
        viewModel.resolveSitePermissionPrompt(firstPrompt.id, allowed = false)

        val secondPrompt = assertNotNull(viewModel.uiState.value.sitePermissionPrompt)
        assertEquals("https://second.example", secondPrompt.origin)
        viewModel.resolveSitePermissionPrompt(secondPrompt.id, allowed = true)

        assertEquals(listOf(false, true), decisions)
        assertNull(viewModel.uiState.value.sitePermissionPrompt)
    }
}
