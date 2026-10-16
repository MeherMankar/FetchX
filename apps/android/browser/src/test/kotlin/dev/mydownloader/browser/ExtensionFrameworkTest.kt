package dev.fetchx.browser

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import org.junit.Test
import kotlin.test.assertFailsWith
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ExtensionFrameworkTest {

    @Test
    fun `extension ID parsing accepts expected identifiers and rejects malformed input`() {
        assertEquals("fetchx-addon@example.org", extensionId().value)
        assertTrue(ExtensionId.parse("").isFailure)
        assertTrue(ExtensionId.parse("../outside").isFailure)
        assertTrue(ExtensionId.parse("bad id").isFailure)
        assertTrue(ExtensionId.parse("x".repeat(129)).isFailure)
    }

    @Test
    fun `permission metadata is normalized deduplicated and malformed values are unsupported`() {
        val permissions = ExtensionPermission.normalize(
            apiPermissions = listOf(" Storage ", "storage", "tabs", "bad permission"),
            origins = listOf("HTTPS://Example.COM/*", "https://example.com/*", "file:///*"),
        )

        assertEquals(4, permissions.size)
        assertTrue(permissions.contains(ExtensionPermission("storage", ExtensionPermissionKind.API)))
        assertTrue(permissions.contains(ExtensionPermission("tabs", ExtensionPermissionKind.API)))
        assertTrue(
            permissions.contains(
                ExtensionPermission("https://example.com/*", ExtensionPermissionKind.HOST_ORIGIN),
            ),
        )
        assertTrue(
            permissions.contains(
                ExtensionPermission("[unsupported permission]", ExtensionPermissionKind.UNSUPPORTED),
            ),
        )
        assertFailsWith<IllegalArgumentException> {
            ExtensionPermission.normalize(List(129) { "tabs" }, emptyList())
        }
    }

    @Test
    fun `permission prompt policy never grants requested APIs origins or private mode`() {
        assertTrue(canInstallWithoutPermissionGrant(emptyArray(), emptyArray()))
        assertFalse(canInstallWithoutPermissionGrant(arrayOf("storage"), emptyArray()))
        assertFalse(canInstallWithoutPermissionGrant(emptyArray(), arrayOf("https://example.com/*")))
    }

    @Test
    fun `only the official uBlock Origin ID may receive bundled extension permissions`() {
        assertTrue(
            canGrantExtensionInstallPermissions(
                extensionId = UBLOCK_ORIGIN_EXTENSION_ID,
                apiPermissions = arrayOf("webRequest", "webRequestBlocking"),
                origins = arrayOf("<all_urls>"),
            ),
        )
        assertFalse(
            canGrantExtensionInstallPermissions(
                extensionId = "other-addon@example.org",
                apiPermissions = arrayOf("webRequest"),
                origins = arrayOf("<all_urls>"),
            ),
        )
        assertTrue(
            canGrantExtensionInstallPermissions(
                extensionId = "permission-free@example.org",
                apiPermissions = emptyArray(),
                origins = emptyArray(),
            ),
        )
    }

    @Test
    fun `extension updates are accepted only when they request no new permissions`() {
        assertTrue(canAcceptExtensionUpdatePermissions(emptyArray(), emptyArray()))
        assertFalse(canAcceptExtensionUpdatePermissions(arrayOf("tabs"), emptyArray()))
        assertFalse(canAcceptExtensionUpdatePermissions(emptyArray(), arrayOf("https://example.com/*")))
    }

    @Test
    fun `Firefox Add-ons installs accept only HTTPS package URLs hosted by AMO`() {
        assertTrue(
            isFirefoxAddonsDownloadUrl(
                "https://addons.mozilla.org/firefox/downloads/file/1234/addon.xpi",
            ),
        )
        assertFalse(isFirefoxAddonsDownloadUrl("http://addons.mozilla.org/firefox/downloads/file/1234/addon.xpi"))
        assertFalse(isFirefoxAddonsDownloadUrl("https://evil.example/firefox/downloads/file/1234/addon.xpi"))
        assertFalse(isFirefoxAddonsDownloadUrl("https://addons.mozilla.org.evil.example/firefox/downloads/file/1234/addon.xpi"))
        assertFalse(isFirefoxAddonsDownloadUrl("https://addons.mozilla.org/en-US/firefox/addon/example/"))
    }

    @Test
    fun `bundled uBlock Origin is classified as non-removable built-in`() {
        assertEquals(
            ExtensionSource.BUILT_IN,
            extensionSource(UBLOCK_ORIGIN_EXTENSION_ID, isBuiltIn = false),
        )
        assertEquals(
            ExtensionSource.LOCAL_PACKAGE,
            extensionSource("local-addon@example.org", isBuiltIn = false),
        )
        assertEquals(
            ExtensionSource.BUILT_IN,
            extensionSource("other-builtin@example.org", isBuiltIn = true),
        )
    }

    @Test
    fun `uBlock enabled state follows the ad blocking setting`() {
        assertTrue(uBlockNeedsStateChange(ExtensionState.DISABLED, adBlockingEnabled = true))
        assertTrue(uBlockNeedsStateChange(ExtensionState.ENABLED, adBlockingEnabled = false))
        assertFalse(uBlockNeedsStateChange(ExtensionState.ENABLED, adBlockingEnabled = true))
        assertFalse(uBlockNeedsStateChange(ExtensionState.DISABLED, adBlockingEnabled = false))
        assertFalse(uBlockNeedsStateChange(ExtensionState.ENABLING, adBlockingEnabled = false))
        assertFalse(uBlockNeedsStateChange(ExtensionState.DISABLING, adBlockingEnabled = true))
    }

    @Test
    fun `store registers searches and returns immutable snapshots`() {
        val store = ExtensionStore()
        val permissions = mutableSetOf(
            ExtensionPermission("storage", ExtensionPermissionKind.API),
        )
        assertTrue(store.register(extension(requestedPermissions = permissions)).isSuccess)
        permissions.clear()

        assertTrue(store.contains(extensionId()))
        assertEquals(1, store.list().size)
        assertEquals(1, store.find(extensionId())?.requestedPermissions?.size)
        assertFailsWith<UnsupportedOperationException> {
            @Suppress("UNCHECKED_CAST")
            (store.list() as MutableList<Extension>).clear()
        }
    }

    @Test
    fun `store rejects duplicate IDs malformed metadata and invalid lifecycle registrations`() {
        val store = ExtensionStore()
        assertTrue(store.register(extension()).isSuccess)
        assertTrue(store.register(extension()).isFailure)
        assertTrue(store.register(extension(name = "  ")).isFailure)
        assertTrue(store.register(extension(state = ExtensionState.INSTALLING)).isFailure)
    }

    @Test
    fun `store removes enables disables and clears extensions`() {
        val store = ExtensionStore()
        val id = extensionId()
        assertTrue(store.register(extension(state = ExtensionState.DISABLED)).isSuccess)
        assertTrue(store.enable(id))
        assertEquals(ExtensionState.ENABLED, store.find(id)?.state)
        assertTrue(store.disable(id))
        assertEquals(ExtensionState.DISABLED, store.find(id)?.state)
        assertNotNull(store.remove(id))
        assertNull(store.find(id))
        assertFalse(store.enable(id))

        store.register(extension("another@example.org"))
        store.clear()
        assertTrue(store.list().isEmpty())
    }

    @Test
    fun `local extension trust state is explicit and separate from permissions`() {
        val store = ExtensionStore()
        val trusted = extension(trust = ExtensionTrustState.TRUSTED_BY_USER)
        assertTrue(store.register(trusted).isSuccess)
        assertEquals(ExtensionTrustState.TRUSTED_BY_USER, store.find(trusted.id)?.trust)
        assertTrue(store.setTrust(trusted.id, ExtensionTrustState.REJECTED))
        assertEquals(ExtensionTrustState.REJECTED, store.find(trusted.id)?.trust)
    }

    @Test
    fun `store list and extension capacity remain bounded`() {
        val store = ExtensionStore()
        val tooMany = (0..128).map { extension("addon$it@example.org") }
        assertTrue(store.replaceAll(tooMany).isFailure)
        assertTrue(store.list().isEmpty())
    }

    @Test
    fun `manager registers installed extensions and serializes enable and disable`() {
        val installed = extension(state = ExtensionState.DISABLED)
        val gateway = FakeGateway(installed = listOf(installed))
        val manager = ExtensionManager(gateway)

        assertEquals(listOf(installed), manager.state.value.extensions)
        assertTrue(manager.enable(installed.id))
        assertEquals(ExtensionState.ENABLED, manager.state.value.extensions.single().state)
        assertTrue(manager.disable(installed.id))
        assertEquals(ExtensionState.DISABLED, manager.state.value.extensions.single().state)
        assertEquals(1, gateway.enableCalls)
        assertEquals(1, gateway.disableCalls)
    }

    @Test
    fun `marketplace permission approval is explicit and never enables private browsing`() {
        val gateway = FakeGateway()
        val manager = ExtensionManager(gateway)
        gateway.deferInstall = true
        assertTrue(
            manager.installFromFirefoxAddons(
                FirefoxAddon(
                    id = "addon@example.org",
                    name = "Example Add-on",
                    version = "1.0",
                    summary = "",
                    downloadUrl = "https://addons.mozilla.org/firefox/downloads/file/1234/addon.xpi",
                ),
            ),
        )
        var approved: Boolean? = null
        gateway.onInstallPermissionPrompt?.invoke(
            ExtensionInstallPermissionRequest(
                extensionId = "addon@example.org",
                name = "Example Add-on",
                permissions = listOf("tabs"),
                origins = listOf("https://example.org/*"),
                dataCollectionPermissions = listOf("technicalAndInteractionData"),
            ),
        ) { allowed ->
            approved = allowed
        }

        assertEquals("addon@example.org", manager.state.value.permissionRequest?.extensionId)
        assertEquals(
            listOf("technicalAndInteractionData"),
            manager.state.value.permissionRequest?.dataCollectionPermissions,
        )
        assertTrue(manager.resolveInstallPermissionRequest(allow = true))
        assertEquals(true, approved)
        assertNull(manager.state.value.permissionRequest)
        gateway.completeInstall()
        manager.shutdown()
    }

    @Test
    fun `manager checks installed extensions for updates during refresh`() {
        val installed = extension(version = "1.0")
        val updated = extension(version = "2.0")
        val gateway = FakeGateway(installed = listOf(installed)).apply {
            updateResults[installed.id] = Result.success(updated)
        }

        val manager = ExtensionManager(gateway)

        assertEquals(listOf(installed.id), gateway.updateCalls)
        assertEquals("2.0", manager.state.value.extensions.single().metadata.version)
        assertNull(manager.state.value.error)
    }

    @Test
    fun `failed automatic update keeps installed extension and surfaces an error`() {
        val installed = extension(version = "1.0")
        val gateway = FakeGateway(installed = listOf(installed)).apply {
            updateResults[installed.id] = Result.failure(IOException("network unavailable"))
        }

        val manager = ExtensionManager(gateway)

        assertEquals(listOf(installed), manager.state.value.extensions)
        assertNotNull(manager.state.value.error)
    }

    @Test
    fun `manager removal deletes user extension but preserves built in extension`() {
        val userExtension = extension()
        val builtInExtension = extension("built-in@example.org", source = ExtensionSource.BUILT_IN)
        val gateway = FakeGateway(installed = listOf(userExtension, builtInExtension))
        val manager = ExtensionManager(gateway)

        assertFalse(manager.remove(builtInExtension.id))
        assertTrue(manager.state.value.extensions.any { it.id == builtInExtension.id })
        assertTrue(manager.remove(userExtension.id))
        assertEquals(listOf(builtInExtension.id), manager.state.value.extensions.map { it.id })
        assertEquals(1, gateway.uninstallCalls)
    }

    @Test
    fun `failed manager operations restore the previous extension state`() {
        val installed = extension(state = ExtensionState.DISABLED)
        val gateway = FakeGateway(installed = listOf(installed)).apply {
            enableResult = Result.failure(IOException("native failure"))
            uninstallResult = Result.failure(IOException("native failure"))
        }
        val manager = ExtensionManager(gateway)

        assertTrue(manager.enable(installed.id))
        assertEquals(ExtensionState.DISABLED, manager.state.value.extensions.single().state)
        assertNotNull(manager.state.value.error)
        assertTrue(manager.remove(installed.id))
        assertEquals(ExtensionState.DISABLED, manager.state.value.extensions.single().state)
        assertNotNull(manager.state.value.error)
    }

    @Test
    fun `manager rejects unknown IDs and refuses overlapping operations`() {
        val installed = extension(state = ExtensionState.DISABLED)
        val gateway = FakeGateway(installed = listOf(installed)).apply {
            deferEnable = true
        }
        val manager = ExtensionManager(gateway)

        assertFalse(manager.enable(extensionId("missing@example.org")))
        assertTrue(manager.enable(installed.id))
        assertEquals(ExtensionState.ENABLING, manager.state.value.extensions.single().state)
        assertFalse(manager.disable(installed.id))
        assertTrue(manager.state.value.isBusy)
        gateway.completeEnable()
        assertFalse(manager.state.value.isBusy)
        assertEquals(ExtensionState.ENABLED, manager.state.value.extensions.single().state)
    }

    @Test
    fun `manager rejects duplicate Gecko IDs without discarding the current registry`() {
        val original = extension()
        val gateway = FakeGateway(installed = listOf(original))
        val manager = ExtensionManager(gateway)
        gateway.listResult = Result.success(listOf(original, original))

        assertTrue(manager.refresh())
        assertEquals(listOf(original), manager.state.value.extensions)
        assertNotNull(manager.state.value.error)
    }

    @Test
    fun `manager install closes staged package after success and failure`() {
        val first = stagedPackage()
        val gateway = FakeGateway().apply { installResult = Result.success(extension()) }
        val manager = ExtensionManager(gateway)

        assertTrue(manager.install(first))
        assertFalse(first.exists())
        assertEquals(extension(), manager.state.value.extensions.single())

        val failedPackage = stagedPackage()
        gateway.installResult = Result.failure(IOException("invalid signature"))
        assertTrue(manager.install(failedPackage))
        assertFalse(failedPackage.exists())
        assertEquals(1, manager.state.value.extensions.size)
        assertNotNull(manager.state.value.error)
    }

    @Test
    fun `installing an existing user extension refreshes its metadata without duplicating it`() {
        val previous = extension(version = "1.0")
        val updated = extension(version = "2.0")
        val gateway = FakeGateway(installed = listOf(previous)).apply {
            installResult = Result.success(updated)
        }
        val manager = ExtensionManager(gateway)

        assertTrue(manager.install(stagedPackage()))

        assertEquals(1, manager.state.value.extensions.size)
        assertEquals("2.0", manager.state.value.extensions.single().metadata.version)
    }

    @Test
    fun `shutdown during installation cleans staged package and ignores late completion`() {
        val packageFile = stagedPackage()
        val gateway = FakeGateway().apply { deferInstall = true }
        val manager = ExtensionManager(gateway)

        assertTrue(manager.install(packageFile))
        assertEquals(ExtensionState.INSTALLING, manager.state.value.operationState)
        manager.shutdown()
        gateway.completeInstall()

        assertFalse(packageFile.exists())
        assertTrue(manager.state.value.isShutdown)
        assertTrue(manager.state.value.extensions.isEmpty())
    }

    @Test
    fun `manager shutdown is idempotent clears references and rejects later changes`() {
        val installed = extension()
        val gateway = FakeGateway(installed = listOf(installed))
        val manager = ExtensionManager(gateway)

        manager.shutdown()
        manager.shutdown()

        assertTrue(manager.state.value.isShutdown)
        assertTrue(manager.state.value.extensions.isEmpty())
        assertFalse(manager.refresh())
        assertEquals(1, gateway.shutdownCalls)
    }

    @Test
    fun `bounded package copy accepts up to limit and rejects oversized content`() {
        val source = ByteArrayInputStream(byteArrayOf(1, 2, 3))
        val copied = ByteArrayOutputStream()
        assertEquals(3L, copyBounded(source, copied, 3))
        assertEquals(listOf<Byte>(1, 2, 3), copied.toByteArray().toList())

        assertFailsWith<IOException> {
            copyBounded(ByteArrayInputStream(byteArrayOf(1, 2, 3)), ByteArrayOutputStream(), 2)
        }
    }

    @Test
    fun `package importer accepts only picker content URIs`() {
        validatePickerContentUri("content", "provider")
        assertFailsWith<SecurityException> { validatePickerContentUri("file", null) }
        assertFailsWith<SecurityException> { validatePickerContentUri("https", "example.com") }
        assertFailsWith<SecurityException> { validatePickerContentUri("content", "") }
    }

    private fun extension(
        id: String = "fetchx-addon@example.org",
        name: String = "FetchX test extension",
        version: String = "1.0",
        state: ExtensionState = ExtensionState.ENABLED,
        source: ExtensionSource = ExtensionSource.LOCAL_PACKAGE,
        requestedPermissions: Set<ExtensionPermission> = emptySet(),
        trust: ExtensionTrustState = ExtensionTrustState.UNVERIFIED,
    ): Extension {
        val metadata = ExtensionMetadata(
            id = extensionId(id),
            name = name,
            version = version,
            description = "A test extension",
        )
        return Extension(metadata, state, source, requestedPermissions, trust)
    }

    private fun extensionId(value: String = "fetchx-addon@example.org"): ExtensionId =
        ExtensionId.parse(value).getOrThrow()

    private fun stagedPackage(): StagedExtensionPackage {
        val file = File.createTempFile("fetchx-extension-test-", ".xpi")
        file.writeBytes(byteArrayOf(1))
        return StagedExtensionPackage(file)
    }

    private inner class FakeGateway(
        installed: List<Extension> = emptyList(),
    ) : GeckoExtensionGateway {
        override var onInstallPermissionPrompt: ((
            ExtensionInstallPermissionRequest,
            (Boolean) -> Unit,
        ) -> Unit)? = null
        var listResult: Result<List<Extension>> = Result.success(installed)
        var installResult: Result<Extension> = Result.success(extension())
        var enableResult: Result<Extension> = Result.success(extension(state = ExtensionState.ENABLED))
        var disableResult: Result<Extension> = Result.success(extension(state = ExtensionState.DISABLED))
        var uninstallResult: Result<Unit> = Result.success(Unit)
        val updateResults = mutableMapOf<ExtensionId, Result<Extension?>>()
        var deferEnable = false
        var deferInstall = false
        var enableCalls = 0
        var disableCalls = 0
        var uninstallCalls = 0
        val updateCalls = mutableListOf<ExtensionId>()
        var shutdownCalls = 0
        private var pendingEnable: ((Result<Extension>) -> Unit)? = null
        private var pendingInstall: ((Result<Extension>) -> Unit)? = null

        override fun list(callback: (Result<List<Extension>>) -> Unit) {
            callback(listResult)
        }

        override fun install(packageUri: String, callback: (Result<Extension>) -> Unit) {
            if (deferInstall) pendingInstall = callback else callback(installResult)
        }

        override fun installFromFirefoxAddons(addon: FirefoxAddon, callback: (Result<Extension>) -> Unit) {
            if (deferInstall) pendingInstall = callback else callback(installResult)
        }

        override fun update(id: ExtensionId, callback: (Result<Extension?>) -> Unit) {
            updateCalls += id
            callback(updateResults[id] ?: Result.success(null))
        }

        override fun enable(id: ExtensionId, callback: (Result<Extension>) -> Unit) {
            enableCalls += 1
            if (deferEnable) pendingEnable = callback else callback(enableResult)
        }

        override fun disable(id: ExtensionId, callback: (Result<Extension>) -> Unit) {
            disableCalls += 1
            callback(disableResult)
        }

        override fun uninstall(id: ExtensionId, callback: (Result<Unit>) -> Unit) {
            uninstallCalls += 1
            callback(uninstallResult)
        }

        override fun shutdown() {
            shutdownCalls += 1
        }

        fun completeEnable() {
            pendingEnable?.invoke(enableResult)
            pendingEnable = null
        }

        fun completeInstall() {
            pendingInstall?.invoke(installResult)
            pendingInstall = null
        }
    }
}
