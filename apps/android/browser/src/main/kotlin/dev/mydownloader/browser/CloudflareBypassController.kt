package dev.fetchx.browser

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

// ── Provider enum ──────────────────────────────────────────────────────────────

/**
 * Anti-bot / CAPTCHA provider that was detected.
 *
 * Mirrors the supported solvers in PyPasser (https://github.com/xHossein/PyPasser) plus
 * the most common interstitial walls encountered in a real browser. In GeckoView the Gecko
 * JS engine handles the challenge automatically; we only need to identify the wall and
 * surface feedback to the user.
 */
enum class AntibotProvider(val displayName: String) {
    CLOUDFLARE("Cloudflare"),
    CLOUDFLARE_TURNSTILE("Cloudflare Turnstile"),
    DDOS_GUARD("DDoS-Guard"),
    ARVAN_CLOUD("ArvanCloud"),
    HCAPTCHA("hCaptcha"),
    RECAPTCHA("reCAPTCHA"),
    IMPERVA("Imperva / Incapsula"),
    SUCURI("Sucuri Firewall"),
    PERIMETERX("PerimeterX"),
    DATADOME("DataDome"),
    ANUBIS("Anubis"),
    KASADA("Kasada"),
    AKAMAI("Akamai"),
    UNKNOWN("Security Check"),
}

// ── State sealed class ─────────────────────────────────────────────────────────

sealed class CloudflareBypassState {
    /** No challenge in progress. */
    object Idle : CloudflareBypassState()
    /** A real URL was loaded; waiting for the title to confirm a challenge. */
    object Detecting : CloudflareBypassState()
    /** An anti-bot wall was confirmed; the browser is working through it. */
    data class Bypassing(val provider: AntibotProvider) : CloudflareBypassState()
    /** The title changed away from the sentinel value — challenge passed. */
    object Passed : CloudflareBypassState()
    /** Bypass timed out or otherwise failed. */
    data class Failed(val provider: AntibotProvider, val reason: String) : CloudflareBypassState()
}

// ── Detection rules ────────────────────────────────────────────────────────────

/**
 * A single title-based detection rule.
 *
 * [prefix] is matched as a case-insensitive prefix of the trimmed page title.
 * [urlFragment] is an optional additional URL substring that must be present for the
 * rule to fire (used when the title alone is too generic).
 */
private data class TitleRule(
    val prefix: String,
    val provider: AntibotProvider,
    val urlFragment: String? = null,
)

/**
 * A URL-only detection rule — fires when the current URL contains [urlFragment],
 * regardless of title.  Used for providers that don't set a distinctive title.
 */
private data class UrlRule(
    val urlFragment: String,
    val provider: AntibotProvider,
)

// All title prefixes are lowercased; matching is case-insensitive and prefix-only.
private val TITLE_RULES: List<TitleRule> = listOf(
    // Cloudflare interstitial ("Just a moment…" / "Attention Required! | Cloudflare")
    TitleRule("just a moment",                    AntibotProvider.CLOUDFLARE),
    TitleRule("attention required! | cloudflare", AntibotProvider.CLOUDFLARE),
    TitleRule("please wait… | cloudflare",        AntibotProvider.CLOUDFLARE),
    TitleRule("one more step",                    AntibotProvider.CLOUDFLARE),
    TitleRule("checking your browser before",     AntibotProvider.CLOUDFLARE),
    // Cloudflare Turnstile embedded widget (full-page variant)
    TitleRule("verifying you are human",          AntibotProvider.CLOUDFLARE_TURNSTILE),
    TitleRule("verify you are human",             AntibotProvider.CLOUDFLARE_TURNSTILE),
    // DDoS-Guard
    TitleRule("ddos-guard",                       AntibotProvider.DDOS_GUARD),
    TitleRule("ddos guard",                       AntibotProvider.DDOS_GUARD),
    // ArvanCloud antibot interstitial
    TitleRule("arvancloud",                       AntibotProvider.ARVAN_CLOUD),
    TitleRule("please wait while we check your",  AntibotProvider.ARVAN_CLOUD),
    // Anubis (open-source antibot wall)
    TitleRule("please wait while your browser is being verified", AntibotProvider.ANUBIS),
    TitleRule("making sure you're not a robot",   AntibotProvider.ANUBIS),
    // hCaptcha managed interstitial / Imperva (uses hCaptcha)
    TitleRule("access denied | hcaptcha",         AntibotProvider.HCAPTCHA),
    TitleRule("human verification",               AntibotProvider.HCAPTCHA),
    // Imperva / Incapsula
    TitleRule("access denied - imperva",          AntibotProvider.IMPERVA),
    TitleRule("access denied | incapsula",        AntibotProvider.IMPERVA),
    TitleRule("incapsula incident",               AntibotProvider.IMPERVA),
    // Sucuri
    TitleRule("sucuri website firewall",          AntibotProvider.SUCURI),
    TitleRule("access denied - sucuri",           AntibotProvider.SUCURI),
    // PerimeterX / HUMAN
    TitleRule("access denied | blocked",          AntibotProvider.PERIMETERX),
    TitleRule("you have been blocked",            AntibotProvider.PERIMETERX),
    // DataDome
    TitleRule("access denied | datadome",         AntibotProvider.DATADOME),
    TitleRule("datadome",                         AntibotProvider.DATADOME),
    // reCAPTCHA full-page challenge
    TitleRule("recaptcha",                        AntibotProvider.RECAPTCHA),
    // Generic (must stay last to avoid shadowing more specific rules)
    TitleRule("checking your browser",            AntibotProvider.UNKNOWN),
    TitleRule("security check",                   AntibotProvider.UNKNOWN),
    TitleRule("please enable javascript",         AntibotProvider.UNKNOWN),
)

private val URL_RULES: List<UrlRule> = listOf(
    // Cloudflare challenge platform URL
    UrlRule("/cdn-cgi/challenge-platform/",  AntibotProvider.CLOUDFLARE),
    // Kasada
    UrlRule("/_ecom/ajax/check_validity",    AntibotProvider.KASADA),
    UrlRule("/kasada/",                      AntibotProvider.KASADA),
    // Akamai Bot Manager
    UrlRule("/_sec/cp_challenge/",           AntibotProvider.AKAMAI),
    UrlRule("/akam/",                        AntibotProvider.AKAMAI),
    // DataDome
    UrlRule("/tmp_dd_",                      AntibotProvider.DATADOME),
    // PerimeterX
    UrlRule("/api/v1/pxchallenge",           AntibotProvider.PERIMETERX),
)

// ── Controller ─────────────────────────────────────────────────────────────────

/**
 * Detects and tracks anti-bot / CAPTCHA interstitial wall state for a single browser tab.
 *
 * Covers all providers supported by PyPasser (https://github.com/xHossein/PyPasser):
 * Cloudflare (interstitial + Turnstile), DDoS-Guard, ArvanCloud, hCaptcha, reCAPTCHA,
 * Imperva, Sucuri, PerimeterX, DataDome, plus Anubis, Akamai, and Kasada.
 *
 * GeckoView's Gecko engine runs real Firefox JS and handles the challenges automatically.
 * This controller purely detects them and surfaces feedback to the user via [state].
 *
 * One instance per tab, created inside [TabManager.buildSession].
 * All methods must be called from the main thread.
 */
class CloudflareBypassController {

    private val _state = MutableStateFlow<CloudflareBypassState>(CloudflareBypassState.Idle)
    val state: StateFlow<CloudflareBypassState> = _state.asStateFlow()

    /**
     * Called from onLocationChange. Transitions [Idle] → [Detecting] for real URLs.
     * Also runs URL rules immediately (some providers are detectable from the URL alone).
     */
    fun onLocationChange(url: String) {
        if (url.isBlank() || url == "about:blank") {
            reset()
            return
        }
        // Check URL-only rules first
        val urlProvider = detectFromUrl(url)
        if (urlProvider != null) {
            _state.value = CloudflareBypassState.Bypassing(urlProvider)
            return
        }
        // If we're already bypassing, don't reset on a redirect mid-challenge
        if (_state.value is CloudflareBypassState.Bypassing) return
        // Otherwise wait for the title
        if (_state.value is CloudflareBypassState.Idle) {
            _state.value = CloudflareBypassState.Detecting
        }
    }

    /**
     * Called from onTitleChange.
     * - Sentinel title while Idle/Detecting → [Bypassing] with the matched provider
     * - Non-sentinel title while Bypassing → [Passed]
     */
    fun onTitleChange(title: String?) {
        val normalised = title?.trim()?.lowercase() ?: return
        val isSentinel = detectFromTitle(normalised) != null

        when (val current = _state.value) {
            is CloudflareBypassState.Idle,
            is CloudflareBypassState.Detecting -> {
                val provider = detectFromTitle(normalised)
                if (provider != null) {
                    _state.value = CloudflareBypassState.Bypassing(provider)
                }
            }
            is CloudflareBypassState.Bypassing -> {
                if (!isSentinel) {
                    _state.value = CloudflareBypassState.Passed
                }
            }
            else -> Unit // Passed / Failed — no further transitions
        }
    }

    /** Marks the bypass as failed (e.g. called from a timeout coroutine in the ViewModel). */
    fun onChallengeFailed(reason: String) {
        val provider = (_state.value as? CloudflareBypassState.Bypassing)?.provider
            ?: AntibotProvider.UNKNOWN
        _state.value = CloudflareBypassState.Failed(provider, reason)
    }

    /** Resets to Idle. Call on tab close, navigation away, or explicit user dismiss. */
    fun reset() {
        _state.value = CloudflareBypassState.Idle
    }

    /** Returns true iff a challenge is currently active (Detecting or Bypassing). */
    fun isActive(): Boolean =
        _state.value is CloudflareBypassState.Detecting ||
            _state.value is CloudflareBypassState.Bypassing

    // ── Detection helpers ──────────────────────────────────────────────────

    private fun detectFromTitle(normalisedTitle: String): AntibotProvider? =
        TITLE_RULES.firstOrNull { normalisedTitle.startsWith(it.prefix) }?.provider

    private fun detectFromUrl(url: String): AntibotProvider? =
        URL_RULES.firstOrNull { url.contains(it.urlFragment, ignoreCase = true) }?.provider
}
