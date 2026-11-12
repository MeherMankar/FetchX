package dev.fetchx.browser

import java.util.Collections
import java.util.Locale

internal const val UBLOCK_ORIGIN_EXTENSION_ID = "uBlock0@raymondhill.net"

/** Extension IDs that are internal plumbing and must never appear in the user-facing UI. */
internal val INTERNAL_EXTENSION_IDS = setOf(
    "video-interceptor@fetchx.dev",
)

@JvmInline
value class ExtensionId private constructor(val value: String) {
    companion object {
        private val validId = Regex("[A-Za-z0-9][A-Za-z0-9._@+-]{0,127}")

        fun parse(value: String): Result<ExtensionId> =
            if (validId.matches(value)) {
                Result.success(ExtensionId(value))
            } else {
                Result.failure(IllegalArgumentException("Invalid extension ID"))
            }
    }
}

data class ExtensionMetadata(
    val id: ExtensionId,
    val name: String,
    val version: String,
    val description: String,
) {
    internal fun validationError(): String? = when {
        name.isBlank() || name.length > MAX_NAME_LENGTH || name.hasControlCharacters() ->
            "Invalid extension name"
        version.isBlank() || version.length > MAX_VERSION_LENGTH || version.hasControlCharacters() ->
            "Invalid extension version"
        description.length > MAX_DESCRIPTION_LENGTH || description.hasControlCharacters() ->
            "Invalid extension description"
        else -> null
    }

    private fun String.hasControlCharacters(): Boolean = any(Char::isISOControl)

    private companion object {
        const val MAX_NAME_LENGTH = 128
        const val MAX_VERSION_LENGTH = 64
        const val MAX_DESCRIPTION_LENGTH = 1024
    }
}

enum class ExtensionState {
    INSTALLING,
    INSTALLED,
    ENABLING,
    ENABLED,
    DISABLING,
    DISABLED,
    REMOVING,
    ERROR,
}

enum class ExtensionSource {
    LOCAL_PACKAGE,
    BUILT_IN,
}

/**
 * Explicit local trust state for imported extensions.
 *
 * This project does not claim a package is cryptographically verified unless a
 * GeckoView API provides a reliable, authoritative signer/trust check. Without
 * such an API, the app must keep the state as an explicit local decision instead
 * of pretending the package is trusted.
 */
enum class ExtensionTrustState {
    UNVERIFIED,
    TRUSTED_BY_USER,
    REJECTED,
}

enum class ExtensionPermissionKind {
    API,
    HOST_ORIGIN,
    UNSUPPORTED,
}

class ExtensionPermission internal constructor(
    val value: String,
    val kind: ExtensionPermissionKind,
) {
    override fun equals(other: Any?): Boolean =
        other is ExtensionPermission && value == other.value && kind == other.kind

    override fun hashCode(): Int = 31 * value.hashCode() + kind.hashCode()

    override fun toString(): String = "ExtensionPermission(value=$value, kind=$kind)"

    companion object {
        private val permissionName = Regex("[a-zA-Z][a-zA-Z0-9._-]{0,127}")
        private val hostOrigin = Regex(
            "https?://(?:\\*\\.)?[a-zA-Z0-9](?:[a-zA-Z0-9.-]{0,251}[a-zA-Z0-9])?(?::[0-9]{1,5})?/\\*",
        )

        fun normalize(apiPermissions: Collection<String>, origins: Collection<String>): Set<ExtensionPermission> {
            require(
                apiPermissions.size <= MAX_PERMISSION_COUNT &&
                    origins.size <= MAX_PERMISSION_COUNT - apiPermissions.size,
            ) {
                "Too many extension permissions"
            }
            val result = LinkedHashSet<ExtensionPermission>()
            apiPermissions.forEach { raw ->
                val value = if (raw.length <= MAX_PERMISSION_LENGTH) raw.trim().lowercase(Locale.ROOT) else ""
                result += if (value.isNotEmpty() && permissionName.matches(value)) {
                    ExtensionPermission(value, ExtensionPermissionKind.API)
                } else {
                    ExtensionPermission(UNSUPPORTED_PERMISSION, ExtensionPermissionKind.UNSUPPORTED)
                }
            }
            origins.forEach { raw ->
                val value = if (raw.length <= MAX_PERMISSION_LENGTH) raw.trim() else ""
                result += if (
                    value.isNotEmpty() &&
                    hostOrigin.matches(value) &&
                    !value.contains("..")
                ) {
                    ExtensionPermission(value.lowercase(Locale.ROOT), ExtensionPermissionKind.HOST_ORIGIN)
                } else {
                    ExtensionPermission(UNSUPPORTED_PERMISSION, ExtensionPermissionKind.UNSUPPORTED)
                }
            }
            return Collections.unmodifiableSet(result)
        }

        private const val MAX_PERMISSION_COUNT = 128
        private const val MAX_PERMISSION_LENGTH = 512
        private const val UNSUPPORTED_PERMISSION = "[unsupported permission]"
    }
}

data class Extension(
    val metadata: ExtensionMetadata,
    val state: ExtensionState,
    val source: ExtensionSource,
    val requestedPermissions: Set<ExtensionPermission> = emptySet(),
    val trust: ExtensionTrustState = ExtensionTrustState.UNVERIFIED,
) {
    val id: ExtensionId get() = metadata.id
}

internal fun canInstallWithoutPermissionGrant(
    apiPermissions: Array<String>,
    origins: Array<String>,
): Boolean = apiPermissions.isEmpty() && origins.isEmpty()

internal fun canGrantExtensionInstallPermissions(
    extensionId: String,
    apiPermissions: Array<String>,
    origins: Array<String>,
): Boolean = canInstallWithoutPermissionGrant(apiPermissions, origins) ||
    extensionId == UBLOCK_ORIGIN_EXTENSION_ID

internal fun canAcceptExtensionUpdatePermissions(
    newPermissions: Array<String>,
    newOrigins: Array<String>,
): Boolean = newPermissions.isEmpty() && newOrigins.isEmpty()

internal fun extensionSource(extensionId: String, isBuiltIn: Boolean): ExtensionSource =
    if (isBuiltIn || extensionId == UBLOCK_ORIGIN_EXTENSION_ID) {
        ExtensionSource.BUILT_IN
    } else {
        ExtensionSource.LOCAL_PACKAGE
    }

internal fun uBlockNeedsStateChange(
    extensionState: ExtensionState,
    uBlockOriginEnabled: Boolean,
): Boolean = when (extensionState) {
    ExtensionState.ENABLED -> !uBlockOriginEnabled
    ExtensionState.DISABLED, ExtensionState.INSTALLED -> uBlockOriginEnabled
    else -> false
}
