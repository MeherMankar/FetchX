package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertTrue

class CloudflareBypassControllerTest {

    // ── Construction ──────────────────────────────────────────────────────

    @Test
    fun `initial state is Idle`() {
        val controller = CloudflareBypassController()
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    // ── onLocationChange ──────────────────────────────────────────────────

    @Test
    fun `onLocationChange with real URL transitions Idle to Detecting`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        assertIs<CloudflareBypassState.Detecting>(controller.state.value)
    }

    @Test
    fun `onLocationChange with about blank resets to Idle`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onLocationChange("about:blank")
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    @Test
    fun `onLocationChange with empty URL resets to Idle`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onLocationChange("")
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    @Test
    fun `onLocationChange does not reset Bypassing state during redirect`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        assertIs<CloudflareBypassState.Bypassing>(controller.state.value)
        // Redirect mid-challenge — should stay Bypassing
        controller.onLocationChange("https://example.com/challenge-result")
        assertIs<CloudflareBypassState.Bypassing>(controller.state.value)
    }

    // ── URL-based detection ───────────────────────────────────────────────

    @Test
    fun `Cloudflare CDN-CGI challenge URL triggers Bypassing immediately`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/cdn-cgi/challenge-platform/h/b/scripts/jsd/main.js")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.CLOUDFLARE, state.provider)
    }

    @Test
    fun `Kasada URL triggers Bypassing immediately`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://shop.example.com/_ecom/ajax/check_validity")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.KASADA, state.provider)
    }

    @Test
    fun `Akamai URL triggers Bypassing immediately`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/_sec/cp_challenge/foo")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.AKAMAI, state.provider)
    }

    // ── Title-based detection — Cloudflare ────────────────────────────────

    @Test
    fun `Cloudflare sentinel title while Detecting transitions to Bypassing`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.CLOUDFLARE, state.provider)
    }

    @Test
    fun `sentinel title matching is case-insensitive`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("JUST A MOMENT")
        assertIs<CloudflareBypassState.Bypassing>(controller.state.value)
    }

    @Test
    fun `attention required title maps to Cloudflare`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Attention Required! | Cloudflare")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.CLOUDFLARE, state.provider)
    }

    // ── Title-based detection — other providers ───────────────────────────

    @Test
    fun `DDoS-Guard sentinel title maps to DDOS_GUARD`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("DDoS-Guard")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.DDOS_GUARD, state.provider)
    }

    @Test
    fun `Cloudflare Turnstile title maps to CLOUDFLARE_TURNSTILE`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Verifying you are human. This may take a few seconds.")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.CLOUDFLARE_TURNSTILE, state.provider)
    }

    @Test
    fun `Anubis title maps to ANUBIS`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Please wait while your browser is being verified…")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.ANUBIS, state.provider)
    }

    @Test
    fun `Sucuri title maps to SUCURI`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Sucuri Website Firewall — Access Denied")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.SUCURI, state.provider)
    }

    @Test
    fun `Imperva title maps to IMPERVA`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Access Denied | Incapsula")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.IMPERVA, state.provider)
    }

    @Test
    fun `DataDome title maps to DATADOME`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Access Denied | DataDome")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.DATADOME, state.provider)
    }

    @Test
    fun `ArvanCloud title maps to ARVAN_CLOUD`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("ArvanCloud - Please wait while we check your browser")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Bypassing>(state)
        assertEquals(AntibotProvider.ARVAN_CLOUD, state.provider)
    }

    // ── Bypassing → Passed transition ─────────────────────────────────────

    @Test
    fun `real title while Bypassing transitions to Passed`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        controller.onTitleChange("Example Domain")
        assertIs<CloudflareBypassState.Passed>(controller.state.value)
    }

    @Test
    fun `non-sentinel title while Detecting stays Detecting`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Home – Example Site")
        assertIs<CloudflareBypassState.Detecting>(controller.state.value)
    }

    // ── onChallengeFailed ─────────────────────────────────────────────────

    @Test
    fun `onChallengeFailed captures provider from Bypassing state`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        controller.onChallengeFailed("Timed out")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Failed>(state)
        assertEquals("Timed out", state.reason)
        assertEquals(AntibotProvider.CLOUDFLARE, state.provider)
    }

    @Test
    fun `onChallengeFailed from Idle uses UNKNOWN provider`() {
        val controller = CloudflareBypassController()
        controller.onChallengeFailed("error")
        val state = controller.state.value
        assertIs<CloudflareBypassState.Failed>(state)
        assertEquals(AntibotProvider.UNKNOWN, state.provider)
    }

    // ── reset ─────────────────────────────────────────────────────────────

    @Test
    fun `reset from Detecting returns to Idle`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.reset()
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    @Test
    fun `reset from Bypassing returns to Idle`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        controller.reset()
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    @Test
    fun `reset from Failed returns to Idle`() {
        val controller = CloudflareBypassController()
        controller.onChallengeFailed("error")
        controller.reset()
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    @Test
    fun `reset from Passed returns to Idle`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        controller.onTitleChange("Real Title")
        controller.reset()
        assertIs<CloudflareBypassState.Idle>(controller.state.value)
    }

    // ── isActive ──────────────────────────────────────────────────────────

    @Test
    fun `isActive returns true for Detecting`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        assertTrue(controller.isActive())
    }

    @Test
    fun `isActive returns true for Bypassing`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        assertTrue(controller.isActive())
    }

    @Test
    fun `isActive returns false for Idle`() {
        assertFalse(CloudflareBypassController().isActive())
    }

    @Test
    fun `isActive returns false for Passed`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        controller.onTitleChange("Real Title")
        assertFalse(controller.isActive())
    }

    @Test
    fun `isActive returns false for Failed`() {
        val controller = CloudflareBypassController()
        controller.onChallengeFailed("error")
        assertFalse(controller.isActive())
    }

    // ── null title ────────────────────────────────────────────────────────

    @Test
    fun `null title is ignored and does not change state`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange(null)
        // Still Detecting — no false positive
        assertIs<CloudflareBypassState.Detecting>(controller.state.value)
    }

    // ── Provider display names ────────────────────────────────────────────

    @Test
    fun `Bypassing state carries correct provider display name`() {
        val controller = CloudflareBypassController()
        controller.onLocationChange("https://example.com/")
        controller.onTitleChange("Just a moment…")
        val state = controller.state.value as CloudflareBypassState.Bypassing
        assertEquals("Cloudflare", state.provider.displayName)
    }
}
