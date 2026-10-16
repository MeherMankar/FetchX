package dev.fetchx.browser

import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import org.json.JSONArray
import org.json.JSONObject

data class FirefoxAddon(
    val id: String,
    val name: String,
    val version: String,
    val summary: String,
    val downloadUrl: String,
)

data class ExtensionInstallPermissionRequest(
    val extensionId: String,
    val name: String,
    val permissions: List<String>,
    val origins: List<String>,
    val dataCollectionPermissions: List<String> = emptyList(),
)

data class FirefoxAddonsCatalogState(
    val query: String = "",
    val results: List<FirefoxAddon> = emptyList(),
    val isLoading: Boolean = false,
    val error: String? = null,
)

internal class FirefoxAddonsCatalog {
    fun search(query: String): List<FirefoxAddon> {
        val normalizedQuery = query.trim()
        require(normalizedQuery.isNotEmpty()) { "Enter an add-on name to search." }
        require(normalizedQuery.length <= MAX_QUERY_LENGTH) { "Search text is too long." }

        val encodedQuery = URLEncoder.encode(normalizedQuery, StandardCharsets.UTF_8.name())
        val requestUrl = URL(
            "$AMO_SEARCH_URL?q=$encodedQuery&type=extension&app=android&lang=en-US&page_size=$PAGE_SIZE",
        )
        val connection = requestUrl.openConnection() as HttpURLConnection
        try {
            connection.requestMethod = "GET"
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = READ_TIMEOUT_MS
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("User-Agent", "FetchX Android")
            if (connection.responseCode !in 200..299) {
                throw IOException("Firefox Add-ons search failed (HTTP ${connection.responseCode}).")
            }
            val body = connection.inputStream.use { input ->
                BufferedInputStream(input).use { buffered ->
                    val output = ByteArrayOutputStream()
                    val chunk = ByteArray(COPY_BUFFER_SIZE)
                    var total = 0
                    while (true) {
                        val count = buffered.read(chunk)
                        if (count < 0) break
                        total += count
                        if (total > MAX_RESPONSE_BYTES) {
                            throw IOException("Firefox Add-ons returned an oversized search response.")
                        }
                        output.write(chunk, 0, count)
                    }
                    output.toString(StandardCharsets.UTF_8.name())
                }
            }
            return try {
                parseResults(JSONObject(body).optJSONArray("results") ?: JSONArray())
            } catch (error: org.json.JSONException) {
                throw IOException("Firefox Add-ons returned invalid search data.", error)
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun parseResults(results: JSONArray): List<FirefoxAddon> =
        (0 until results.length()).mapNotNull { index ->
            val addon = results.optJSONObject(index) ?: return@mapNotNull null
            val currentVersion = addon.optJSONObject("current_version") ?: return@mapNotNull null
            val file = currentVersion.optJSONObject("file") ?: return@mapNotNull null
            val url = file.optString("url")
            if (!isFirefoxAddonsDownloadUrl(url)) return@mapNotNull null

            val id = addon.optString("guid").takeIf(String::isNotBlank)
                ?: addon.optString("id").takeIf(String::isNotBlank)
                ?: return@mapNotNull null
            val name = localizedValue(addon.optJSONObject("name"))
                .takeIf(String::isNotBlank)
                ?: return@mapNotNull null
            val summary = localizedValue(addon.optJSONObject("summary"))
                .takeIf(String::isNotBlank)
                .orEmpty()
                .take(MAX_SUMMARY_LENGTH)
            FirefoxAddon(
                id = id,
                name = name.take(MAX_NAME_LENGTH),
                version = currentVersion.optString("version").take(MAX_VERSION_LENGTH),
                summary = summary,
                downloadUrl = url,
            )
        }

    private fun localizedValue(value: JSONObject?): String =
        value?.optString("en-US")?.takeIf(String::isNotBlank)
            ?: value?.keys()?.asSequence()?.mapNotNull { key ->
                value.optString(key).takeIf(String::isNotBlank)
            }?.firstOrNull()
            .orEmpty()

    private companion object {
        const val AMO_SEARCH_URL = "https://addons.mozilla.org/api/v5/addons/search/"
        const val PAGE_SIZE = 12
        const val MAX_QUERY_LENGTH = 100
        const val MAX_RESPONSE_BYTES = 1_000_000
        const val MAX_NAME_LENGTH = 128
        const val MAX_VERSION_LENGTH = 64
        const val MAX_SUMMARY_LENGTH = 512
        const val CONNECT_TIMEOUT_MS = 10_000
        const val READ_TIMEOUT_MS = 15_000
        const val COPY_BUFFER_SIZE = 8 * 1024
    }
}

internal fun isFirefoxAddonsDownloadUrl(value: String): Boolean = try {
    val url = URL(value)
    url.protocol == "https" &&
        url.host.equals("addons.mozilla.org", ignoreCase = true) &&
        url.path.startsWith("/firefox/downloads/file/") &&
        url.userInfo == null &&
        url.port in setOf(-1, 443)
} catch (_: java.net.MalformedURLException) {
    false
}
