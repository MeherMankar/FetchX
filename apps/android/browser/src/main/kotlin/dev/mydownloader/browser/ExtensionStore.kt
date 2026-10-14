package dev.fetchx.browser

import java.util.Collections

class ExtensionStore {
    private val lock = Any()
    private val extensions = LinkedHashMap<ExtensionId, Extension>()

    fun register(extension: Extension): Result<Unit> = synchronized(lock) {
        extension.validationError()?.let {
            return@synchronized Result.failure(IllegalArgumentException(it))
        }
        if (extension.state.isTransient()) {
            return@synchronized Result.failure(IllegalArgumentException("Invalid stable extension state"))
        }
        if (extensions.containsKey(extension.id)) {
            return@synchronized Result.failure(IllegalArgumentException("Duplicate extension ID"))
        }
        if (extensions.size >= MAX_EXTENSION_COUNT) {
            return@synchronized Result.failure(IllegalStateException("Extension limit reached"))
        }
        extensions[extension.id] = extension.immutableCopy()
        Result.success(Unit)
    }

    fun remove(id: ExtensionId): Extension? = synchronized(lock) {
        extensions.remove(id)
    }

    fun find(id: ExtensionId): Extension? = synchronized(lock) {
        extensions[id]
    }

    fun list(): List<Extension> = synchronized(lock) {
        Collections.unmodifiableList(ArrayList(extensions.values))
    }

    fun enable(id: ExtensionId): Boolean = synchronized(lock) {
        val extension = extensions[id] ?: return@synchronized false
        when (extension.state) {
            ExtensionState.ENABLED -> true
            ExtensionState.DISABLED, ExtensionState.INSTALLED ->
                replaceLocked(extension.copy(state = ExtensionState.ENABLED))
            else -> false
        }
    }

    fun disable(id: ExtensionId): Boolean = synchronized(lock) {
        val extension = extensions[id] ?: return@synchronized false
        when (extension.state) {
            ExtensionState.DISABLED -> true
            ExtensionState.ENABLED, ExtensionState.INSTALLED ->
                replaceLocked(extension.copy(state = ExtensionState.DISABLED))
            else -> false
        }
    }

    fun contains(id: ExtensionId): Boolean = synchronized(lock) {
        extensions.containsKey(id)
    }

    fun clear() = synchronized(lock) {
        extensions.clear()
    }

    fun setTrust(id: ExtensionId, trust: ExtensionTrustState): Boolean = synchronized(lock) {
        val extension = extensions[id] ?: return@synchronized false
        replaceLocked(extension.copy(trust = trust))
    }

    internal fun replace(extension: Extension): Boolean = synchronized(lock) {
        if (!extensions.containsKey(extension.id) || extension.validationError() != null) {
            return@synchronized false
        }
        replaceLocked(extension.immutableCopy())
    }

    internal fun replaceAll(values: List<Extension>): Result<Unit> = synchronized(lock) {
        if (values.size > MAX_EXTENSION_COUNT) {
            return@synchronized Result.failure(IllegalStateException("Extension limit reached"))
        }
        val replacement = LinkedHashMap<ExtensionId, Extension>()
        for (extension in values) {
            extension.validationError()?.let {
                return@synchronized Result.failure(IllegalArgumentException(it))
            }
            if (extension.state.isTransient()) {
                return@synchronized Result.failure(IllegalArgumentException("Invalid stable extension state"))
            }
            if (replacement.put(extension.id, extension.immutableCopy()) != null) {
                return@synchronized Result.failure(IllegalArgumentException("Duplicate extension ID"))
            }
        }
        extensions.clear()
        extensions.putAll(replacement)
        Result.success(Unit)
    }

    private fun replaceLocked(extension: Extension): Boolean {
        extensions[extension.id] = extension
        return true
    }

    private fun Extension.validationError(): String? = when {
        metadata.validationError() != null -> metadata.validationError()
        requestedPermissions.size > MAX_PERMISSION_COUNT -> "Too many extension permissions"
        else -> null
    }

    private fun ExtensionState.isTransient(): Boolean = this == ExtensionState.INSTALLING ||
        this == ExtensionState.REMOVING || this == ExtensionState.ENABLING || this == ExtensionState.DISABLING

    private fun Extension.immutableCopy(): Extension = copy(
        requestedPermissions = Collections.unmodifiableSet(LinkedHashSet(requestedPermissions)),
    )

    private companion object {
        const val MAX_PERMISSION_COUNT = 128
        const val MAX_EXTENSION_COUNT = 128
    }
}
