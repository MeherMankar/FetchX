package dev.fetchx.bridge

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.fetchx.bridge.uniffi.MobileApiException
import dev.fetchx.bridge.uniffi.PluginSecurityService
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.security.MessageDigest

@RunWith(AndroidJUnit4::class)
class PluginPolicyInstrumentedTest {

    @Test
    fun androidBridgeDeniesUnknownUntrustedAndRevokedPlugins() {
        loadBridge()
        val manifest = manifestJson()
        for ((trust, expectedReason) in listOf(
            "unknown" to "trust_unknown",
            "untrusted" to "plugin_untrusted",
            "revoked" to "trust_revoked",
        )) {
            val decision = evaluate(manifest, trustContext(manifest, trust = trust))
            assertDenied(decision, expectedReason)
        }
    }

    @Test
    fun androidBridgeRequiresPermissionAndCapabilityApprovalAndRuntimeSupport() {
        loadBridge()
        val manifest = manifestJson()

        assertDenied(
            evaluate(manifest, trustContext(manifest, approvedPermissions = "[]")),
            "permission_not_approved",
        )
        assertDenied(
            evaluate(manifest, trustContext(manifest, approvedCapabilities = "[]")),
            "capability_not_approved",
        )
        assertDenied(
            evaluate(manifest, trustContext(manifest)),
            "capability_unsupported",
        )
    }

    @Test
    fun androidBridgeBindsApprovalToManifestAndPlatform() {
        loadBridge()
        val manifest = manifestJson()
        assertDenied(
            evaluate(
                manifest,
                trustContext(manifest, fingerprint = "00".repeat(32)),
            ),
            "manifest_changed",
        )
        assertDenied(
            evaluate(
                manifestJson(platforms = """["windows"]"""),
                trustContext(manifestJson(platforms = """["windows"]""")),
            ),
            "unsupported_platform",
        )
    }

    @Test
    fun androidBridgeRejectsMalformedManifestAndUndeclaredPermission() {
        loadBridge()
        assertValidationError("{}", "{}")

        val manifestWithMissingPermission = manifestJson(
            permissions = "[]",
        )
        assertValidationError(
            manifestWithMissingPermission,
            trustContext(manifestWithMissingPermission),
        )
    }

    private fun loadBridge() {
        System.loadLibrary("fetchx_android_bridge")
    }

    private fun manifestJson(
        permissions: String = """["downloads"]""",
        platforms: String = "[]",
    ): String =
        """{"manifest_version":1,"id":"my-plugin","name":"Test","version":"1.0.0","api_version":1,"compatibility":{"minimum_api_version":1,"maximum_api_version":1},"publisher":"Test Publisher","description":"Instrumented policy fixture","executable":"/plugins/test/plugin","permissions":$permissions,"capabilities":["download_create"],"platforms":$platforms,"signature":null}"""

    private fun trustContext(
        manifest: String,
        trust: String = "trusted",
        approvedPermissions: String = """["downloads"]""",
        approvedCapabilities: String = """["download_create"]""",
        fingerprint: String = sha256(manifest),
    ): String =
        """{"trust":"$trust","signature_status":"unsigned","approved_permissions":$approvedPermissions,"approved_capabilities":$approvedCapabilities,"manifest_sha256":"$fingerprint"}"""

    private fun evaluate(manifest: String, trustContext: String): JSONObject =
        JSONObject(PluginSecurityService().evaluatePolicy(manifest, trustContext))

    private fun assertDenied(decision: JSONObject, reason: String) {
        assertEquals(0, decision.getJSONArray("granted_capabilities").length())
        assertEquals(
            reason,
            decision.getJSONArray("denied_capabilities")
                .getJSONObject(0)
                .getString("reason"),
        )
    }

    private fun assertValidationError(manifest: String, trustContext: String) {
        try {
            PluginSecurityService().evaluatePolicy(manifest, trustContext)
            throw AssertionError("Invalid input was accepted")
        } catch (error: MobileApiException.Validation) {
            assertTrue(error.message.orEmpty().isNotBlank())
        }
    }

    private fun sha256(value: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray(Charsets.UTF_8))
            .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
}
