package dev.fetchx.browser

import java.io.IOException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class ExtensionManagerState(
    val extensions: List<Extension> = emptyList(),
    val isLoading: Boolean = false,
    val isBusy: Boolean = false,
    val operationState: ExtensionState? = null,
    val error: String? = null,
    val isShutdown: Boolean = false,
)

internal interface GeckoExtensionGateway {
    fun list(callback: (Result<List<Extension>>) -> Unit)
    fun install(packageUri: String, callback: (Result<Extension>) -> Unit)
    fun enable(id: ExtensionId, callback: (Result<Extension>) -> Unit)
    fun disable(id: ExtensionId, callback: (Result<Extension>) -> Unit)
    fun uninstall(id: ExtensionId, callback: (Result<Unit>) -> Unit)
    fun shutdown()
}

class ExtensionManager internal constructor(
    private val gateway: GeckoExtensionGateway,
    private val store: ExtensionStore = ExtensionStore(),
) {
    private val lock = Any()
    private val stagedPackages = LinkedHashSet<StagedExtensionPackage>()
    private val _state = MutableStateFlow(ExtensionManagerState(isLoading = true))
    val state: StateFlow<ExtensionManagerState> = _state.asStateFlow()

    @Volatile
    var onStateChanged: (() -> Unit)? = null

    init {
        refresh()
    }

    fun refresh(): Boolean {
        if (!beginOperation(loading = true)) return false
        gateway.list { result ->
            complete {
                result.fold(
                    onSuccess = { extensions ->
                        val sorted = extensions.sortedBy { it.id.value }
                        val replacement = store.replaceAll(sorted)
                        if (replacement.isSuccess) {
                            clearError()
                        } else {
                            setError("The installed extension list contained invalid metadata.")
                        }
                    },
                    onFailure = { setError("Could not read installed extensions.") },
                )
            }
        }
        return true
    }

    internal fun install(packageFile: StagedExtensionPackage): Boolean {
        if (!beginOperation(operationState = ExtensionState.INSTALLING)) {
            if (!closePackage(packageFile)) reject("The staged extension package could not be removed.")
            return false
        }
        val managerShutDown = synchronized(lock) {
            if (_state.value.isShutdown) {
                true
            } else {
                stagedPackages += packageFile
                false
            }
        }
        if (managerShutDown) {
            if (!closePackage(packageFile)) {
                synchronized(lock) {
                    if (_state.value.isShutdown) {
                        _state.value = _state.value.copy(
                            error = "The staged extension package could not be removed from cache.",
                        )
                    }
                }
                publish()
            }
            return false
        }
        if (store.list().size >= MAX_EXTENSION_COUNT) {
            val packageCleaned = closePackage(packageFile)
            synchronized(lock) { stagedPackages.remove(packageFile) }
            finishWithError(
                if (packageCleaned) "The extension limit has been reached."
                else "The extension limit has been reached and the staged package could not be removed.",
            )
            return false
        }
        gateway.install(packageFile.uri) { result ->
            synchronized(lock) {
                stagedPackages.remove(packageFile)
            }
            val packageCleaned = closePackage(packageFile)
            complete {
                result.fold(
                    onSuccess = { extension ->
                        val existing = store.find(extension.id)
                        val stored = if (existing == null) {
                            store.register(extension).isSuccess
                        } else {
                            existing.source == extension.source && store.replace(extension)
                        }
                        if (stored) {
                            clearError()
                        } else {
                            setError("The installed extension returned conflicting metadata.")
                        }
                    },
                    onFailure = {
                        setError(
                            "Installation failed. Check the package signature and ensure it requests no permissions; permission review is not available yet.",
                        )
                    },
                )
                if (!packageCleaned) {
                    setError("The staged extension package could not be removed from cache.")
                }
            }
        }
        return true
    }

    fun enable(id: ExtensionId): Boolean = changeEnabledState(id, enabling = true)

    fun disable(id: ExtensionId): Boolean = changeEnabledState(id, enabling = false)

    fun remove(id: ExtensionId): Boolean {
        val extension = store.find(id) ?: return reject("Extension is no longer installed.")
        if (extension.source == ExtensionSource.BUILT_IN) {
            return reject("Built-in extensions cannot be removed.")
        }
        if (extension.state !in STABLE_MANAGED_STATES) {
            return reject("This extension cannot be removed in its current state.")
        }
        if (!beginOperation(operationState = ExtensionState.REMOVING)) return false
        if (!store.replace(extension.copy(state = ExtensionState.REMOVING))) {
            finishWithError("The extension state changed before removal.")
            return false
        }
        publishStoreState()
        gateway.uninstall(id) { result ->
            complete {
                result.fold(
                    onSuccess = {
                        store.remove(id)
                        clearError()
                    },
                    onFailure = {
                        store.replace(extension)
                        setError("Could not remove the extension.")
                    },
                )
            }
        }
        return true
    }

    internal fun reportImportFailure(cleanupFailed: Boolean = false) {
        synchronized(lock) {
            if (_state.value.isShutdown) return
            _state.value = _state.value.copy(
                error = if (cleanupFailed) {
                    "The staged extension package could not be removed from cache."
                } else {
                    "Could not read the selected local extension package."
                },
            )
        }
        publish()
    }

    fun shutdown() {
        val packages = synchronized(lock) {
            if (_state.value.isShutdown) return
            val pending = stagedPackages.toList()
            stagedPackages.clear()
            store.clear()
            _state.value = ExtensionManagerState(isShutdown = true)
            pending
        }
        gateway.shutdown()
        val cleanupFailed = packages.any { !closePackage(it) }
        if (cleanupFailed) {
            synchronized(lock) {
                _state.value = _state.value.copy(
                    error = "A staged extension package could not be removed from cache.",
                )
            }
        }
        publish()
        onStateChanged = null
    }

    private fun changeEnabledState(id: ExtensionId, enabling: Boolean): Boolean {
        val extension = store.find(id) ?: return reject("Extension is no longer installed.")
        if (enabling && extension.state == ExtensionState.ENABLED) return true
        if (!enabling && extension.state == ExtensionState.DISABLED) return true
        val allowedStates = if (enabling) {
            setOf(ExtensionState.DISABLED, ExtensionState.INSTALLED)
        } else {
            setOf(ExtensionState.ENABLED, ExtensionState.INSTALLED)
        }
        if (extension.state !in allowedStates) {
            return reject("This extension cannot be changed in its current state.")
        }
        val transitionalState = if (enabling) ExtensionState.ENABLING else ExtensionState.DISABLING
        if (!beginOperation(operationState = transitionalState)) return false
        if (!store.replace(extension.copy(state = transitionalState))) {
            finishWithError("The extension state changed before the operation.")
            return false
        }
        publishStoreState()
        val action: (ExtensionId, (Result<Extension>) -> Unit) -> Unit =
            if (enabling) gateway::enable else gateway::disable
        action(id) { result ->
            complete {
                result.fold(
                    onSuccess = { updated ->
                        val expectedState = if (enabling) ExtensionState.ENABLED else ExtensionState.DISABLED
                        if (updated.id != id || updated.state != expectedState || !store.replace(updated)) {
                            store.replace(extension)
                            setError("GeckoView returned invalid extension state.")
                        } else {
                            clearError()
                        }
                    },
                    onFailure = {
                        store.replace(extension)
                        setError(if (enabling) "Could not enable the extension." else "Could not disable the extension.")
                    },
                )
            }
        }
        return true
    }

    private fun beginOperation(
        loading: Boolean = false,
        operationState: ExtensionState? = null,
    ): Boolean {
        var started = false
        synchronized(lock) {
            val current = _state.value
            if (!current.isShutdown) {
                _state.value = if (current.isBusy) {
                    current.copy(error = "Another extension operation is in progress.")
                } else {
                    started = true
                    current.copy(
                        isBusy = true,
                        isLoading = loading,
                        operationState = operationState,
                        error = null,
                    )
                }
            }
        }
        publish()
        return started
    }

    private fun complete(update: () -> Unit) {
        synchronized(lock) {
            if (_state.value.isShutdown) return
            update()
            _state.value = _state.value.copy(
                extensions = store.list(),
                isLoading = false,
                isBusy = false,
                operationState = null,
            )
        }
        publish()
    }

    private fun reject(message: String): Boolean {
        synchronized(lock) {
            if (_state.value.isShutdown) return false
            _state.value = _state.value.copy(error = message)
        }
        publish()
        return false
    }

    private fun finishWithError(message: String) {
        synchronized(lock) {
            if (_state.value.isShutdown) return
            _state.value = _state.value.copy(
                isBusy = false,
                isLoading = false,
                operationState = null,
                error = message,
            )
        }
        publish()
    }

    private fun publish() {
        onStateChanged?.invoke()
    }

    private fun publishStoreState() {
        synchronized(lock) {
            _state.value = _state.value.copy(extensions = store.list())
        }
        publish()
    }

    private fun closePackage(packageFile: StagedExtensionPackage): Boolean = try {
        packageFile.close()
        true
    } catch (error: IOException) {
        false
    }

    private fun clearError() {
        synchronized(lock) {
            _state.value = _state.value.copy(error = null)
        }
    }

    private fun setError(message: String) {
        synchronized(lock) {
            _state.value = _state.value.copy(error = message)
        }
    }

    private companion object {
        const val MAX_EXTENSION_COUNT = 128
        val STABLE_MANAGED_STATES = setOf(
            ExtensionState.INSTALLED,
            ExtensionState.ENABLED,
            ExtensionState.DISABLED,
            ExtensionState.ERROR,
        )
    }
}
