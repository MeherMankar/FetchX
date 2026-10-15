package dev.fetchx.browser

import android.util.Log
import org.mozilla.geckoview.AllowOrDeny
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.WebExtension
import org.mozilla.geckoview.WebExtensionController

private const val TAG = "GeckoExtensions"
private const val UBLOCK_PACKAGE_URI = "resource://android/assets/ublock.xpi"

internal class GeckoExtensionGatewayImpl(
    private val controller: WebExtensionController,
) : GeckoExtensionGateway {
    private val lock = Any()
    private val handles = LinkedHashMap<ExtensionId, WebExtension>()
    private var isShutdown = false
    private var activeInstall: GeckoResult<WebExtension>? = null

    init {
        controller.setPromptDelegate(object : WebExtensionController.PromptDelegate {
            override fun onInstallPromptRequest(
                extension: WebExtension,
                permissions: Array<String>,
                origins: Array<String>,
            ): GeckoResult<WebExtension.PermissionPromptResponse> {
                val canGrantPermissions = canGrantExtensionInstallPermissions(
                    extensionId = extension.id,
                    apiPermissions = permissions,
                    origins = origins,
                )
                return GeckoResult.fromValue(
                    WebExtension.PermissionPromptResponse(canGrantPermissions, false),
                )
            }

            override fun onUpdatePrompt(
                currentlyInstalled: WebExtension,
                updatedExtension: WebExtension,
                newPermissions: Array<String>,
                newOrigins: Array<String>,
            ): GeckoResult<AllowOrDeny> = GeckoResult.fromValue(
                if (canAcceptExtensionUpdatePermissions(newPermissions, newOrigins)) {
                    AllowOrDeny.ALLOW
                } else {
                    AllowOrDeny.DENY
                },
            )

            override fun onOptionalPrompt(
                extension: WebExtension,
                permissions: Array<String>,
                origins: Array<String>,
            ): GeckoResult<AllowOrDeny> = GeckoResult.fromValue(AllowOrDeny.DENY)
        })
    }

    override fun list(callback: (Result<List<Extension>>) -> Unit) {
        controller.list().accept(
            { installed ->
                if (installed == null) {
                    callback(Result.failure(IllegalStateException("GeckoView returned no extension list")))
                    return@accept
                }
                if (installed.none { it.id == UBLOCK_ORIGIN_EXTENSION_ID }) {
                    installBundledUBlock(installed, callback)
                } else {
                    publishInstalled(installed, callback)
                }
            },
            { error ->
                fail("list", error, callback)
            },
        )
    }

    private fun installBundledUBlock(
        installed: List<WebExtension>,
        callback: (Result<List<Extension>>) -> Unit,
    ) {
        val canInstall = synchronized(lock) { !isShutdown }
        if (!canInstall) {
            callback(Result.failure(IllegalStateException("Extension manager is shut down")))
            return
        }
        controller.install(UBLOCK_PACKAGE_URI, WebExtensionController.INSTALLATION_METHOD_FROM_FILE).accept(
            { extension ->
                if (extension == null) {
                    callback(Result.failure(IllegalStateException("GeckoView returned no installed uBlock Origin extension")))
                    return@accept
                }
                val conversion = extension.toDomain()
                val conversionError = conversion.exceptionOrNull()
                if (conversionError != null) {
                    discardMalformedInstall(extension, conversionError) { result ->
                        callback(
                            Result.failure(
                                result.exceptionOrNull()
                                    ?: conversionError,
                            ),
                        )
                    }
                    return@accept
                }
                Log.i(TAG, "Bundled uBlock Origin installed")
                publishInstalled(installed + extension, callback)
            },
            { error ->
                fail("install bundled uBlock Origin", error, callback)
            },
        )
    }

    private fun publishInstalled(
        installed: List<WebExtension>,
        callback: (Result<List<Extension>>) -> Unit,
    ) {
        val convertedResults = installed.map { it.toDomain() }
        val conversionError = convertedResults.firstNotNullOfOrNull { it.exceptionOrNull() }
        if (conversionError != null) {
            Log.e(TAG, "GeckoView returned malformed extension metadata")
            callback(Result.failure(conversionError))
            return
        }
        val converted = convertedResults.mapNotNull { it.getOrNull() }
        synchronized(lock) {
            if (!isShutdown) {
                handles.clear()
                installed.forEach { extension ->
                    ExtensionId.parse(extension.id).getOrNull()?.let { id ->
                        handles[id] = extension
                    }
                }
            }
        }
        callback(Result.success(converted))
    }

    override fun install(packageUri: String, callback: (Result<Extension>) -> Unit) {
        val operation = synchronized(lock) {
            if (isShutdown) {
                null
            } else {
                controller.install(
                    packageUri,
                    WebExtensionController.INSTALLATION_METHOD_FROM_FILE,
                ).also { activeInstall = it }
            }
        }
        if (operation == null) {
            callback(Result.failure(IllegalStateException("Extension manager is shut down")))
            return
        }
        operation.accept(
            { extension ->
                synchronized(lock) {
                    if (activeInstall === operation) activeInstall = null
                }
                if (extension == null) {
                    callback(Result.failure(IllegalStateException("GeckoView returned no installed extension")))
                    return@accept
                }
                val result = extension.toDomain()
                result.fold(
                    onSuccess = { domain ->
                        synchronized(lock) {
                            if (!isShutdown) handles[domain.id] = extension
                        }
                        callback(Result.success(domain))
                    },
                    onFailure = { conversionError ->
                        discardMalformedInstall(extension, conversionError, callback)
                    }
                )
            },
            { error ->
                synchronized(lock) {
                    if (activeInstall === operation) activeInstall = null
                }
                fail("install", error, callback)
            },
        )
    }

    override fun update(id: ExtensionId, callback: (Result<Extension?>) -> Unit) {
        val extension = synchronized(lock) { handles[id] }
        if (extension == null) {
            callback(Result.failure(IllegalStateException("Extension handle is unavailable")))
            return
        }
        if (extension.isBuiltIn) {
            callback(Result.success(null))
            return
        }
        controller.update(extension).accept(
            { updated ->
                if (updated == null) {
                    callback(Result.success(null))
                } else {
                    updateHandle(updated) { result ->
                        callback(result.map { it })
                    }
                }
            },
            { error -> fail("update", error, callback) },
        )
    }

    override fun enable(id: ExtensionId, callback: (Result<Extension>) -> Unit) {
        val extension = synchronized(lock) { handles[id] }
        if (extension == null) {
            callback(Result.failure(IllegalStateException("Extension handle is unavailable")))
            return
        }
        controller.enable(extension, WebExtensionController.EnableSource.USER).accept(
            { updated ->
                if (updated == null) callback(Result.failure(IllegalStateException("GeckoView returned no extension")))
                else updateHandle(updated, callback)
            },
            { error -> fail("enable", error, callback) },
        )
    }

    override fun disable(id: ExtensionId, callback: (Result<Extension>) -> Unit) {
        val extension = synchronized(lock) { handles[id] }
        if (extension == null) {
            callback(Result.failure(IllegalStateException("Extension handle is unavailable")))
            return
        }
        controller.disable(extension, WebExtensionController.EnableSource.USER).accept(
            { updated ->
                if (updated == null) callback(Result.failure(IllegalStateException("GeckoView returned no extension")))
                else updateHandle(updated, callback)
            },
            { error -> fail("disable", error, callback) },
        )
    }

    override fun uninstall(id: ExtensionId, callback: (Result<Unit>) -> Unit) {
        val extension = synchronized(lock) { handles[id] }
        if (extension == null) {
            callback(Result.failure(IllegalStateException("Extension handle is unavailable")))
            return
        }
        controller.uninstall(extension).accept(
            {
                synchronized(lock) { handles.remove(id) }
                callback(Result.success(Unit))
            },
            { error -> fail("remove", error, callback) },
        )
    }

    override fun shutdown() {
        val pendingInstall = synchronized(lock) {
            if (isShutdown) return
            isShutdown = true
            handles.clear()
            activeInstall.also { activeInstall = null }
        }
        pendingInstall?.cancel()
        controller.setPromptDelegate(null)
    }

    private fun updateHandle(
        extension: WebExtension,
        callback: (Result<Extension>) -> Unit,
    ) {
        val result = extension.toDomain()
        result.onSuccess { domain ->
            synchronized(lock) {
                if (!isShutdown) handles[domain.id] = extension
            }
        }
        callback(result)
    }

    private fun discardMalformedInstall(
        extension: WebExtension,
        conversionError: Throwable,
        callback: (Result<Extension>) -> Unit,
    ) {
        controller.uninstall(extension).accept(
            {
                callback(Result.failure(conversionError))
            },
            { cleanupError ->
                val cause = cleanupError ?: IllegalStateException("GeckoView returned an empty cleanup error")
                Log.e(TAG, "Could not remove malformed installed extension (${cause.javaClass.simpleName})")
                callback(Result.failure(conversionError))
            },
        )
    }

    private fun <T> fail(
        operation: String,
        error: Throwable?,
        callback: (Result<T>) -> Unit,
    ) {
        val cause = error ?: IllegalStateException("GeckoView returned an empty error")
        Log.e(TAG, "GeckoView could not $operation extension (${cause.javaClass.simpleName})")
        callback(Result.failure(cause))
    }
}

private fun WebExtension.toDomain(): Result<Extension> = try {
    val extensionId = ExtensionId.parse(id).getOrElse { return Result.failure(it) }
    val metadata = ExtensionMetadata(
        id = extensionId,
        name = metaData.name.orEmpty(),
        version = metaData.version.orEmpty(),
        description = metaData.description.orEmpty(),
    )
    val metadataError = metadata.validationError()
    if (metadataError != null) {
        Result.failure(IllegalArgumentException(metadataError))
    } else {
        val permissions = ExtensionPermission.normalize(
            apiPermissions = metaData.requiredPermissions.orEmpty().toList(),
            origins = metaData.requiredOrigins.orEmpty().toList(),
        )
        Result.success(
            Extension(
                metadata = metadata,
                state = if (metaData.enabled) ExtensionState.ENABLED else ExtensionState.DISABLED,
                source = extensionSource(id, isBuiltIn),
                requestedPermissions = permissions,
            ),
        )
    }
} catch (error: IllegalArgumentException) {
    Result.failure(error)
}
