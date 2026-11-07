package dev.fetchx.app.aria2

import android.content.Context
import org.json.JSONObject
import java.io.BufferedReader
import java.net.HttpURLConnection
import java.net.URL

data class Aria2RemoteConfig(
    val url: String,
    val secret: String,
)

internal object Aria2RemotePreferences {
    const val PREFERENCES_NAME = "fetchx_aria2_remote"
    const val URL_KEY = "rpc_url"
    const val SECRET_KEY = "rpc_secret"

    fun load(context: Context): Aria2RemoteConfig {
        val preferences = context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
        return Aria2RemoteConfig(
            url = preferences.getString(URL_KEY, "") ?: "",
            secret = preferences.getString(SECRET_KEY, "") ?: "",
        )
    }

    fun save(context: Context, config: Aria2RemoteConfig) {
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .edit()
            .putString(URL_KEY, config.url.trim())
            .putString(SECRET_KEY, config.secret.trim())
            .apply()
    }
}

internal fun normalizeRpcUrl(url: String): String {
    val trimmed = url.trim()
    require(trimmed.isNotEmpty()) { "Enter an aria2 RPC URL" }
    val parsed = URL(trimmed)
    require(parsed.protocol == "http" || parsed.protocol == "https") {
        "RPC URL must use HTTP or HTTPS"
    }
    require(parsed.host.isNotBlank()) { "RPC URL must include a host" }
    return trimmed.trimEnd('/')
}

internal fun aria2RpcRequest(method: String, secret: String, params: List<String> = emptyList()): String {
    val requestParams = buildList {
        if (secret.isNotBlank()) add("token:$secret")
        addAll(params)
    }
    val paramsJson = requestParams.joinToString(prefix = "[", postfix = "]", separator = ",") {
        "\"" + it.replace("\\", "\\\\")
            .replace("\"", "\\\"")
            .replace("\n", "\\n")
            .replace("\r", "\\r")
            .replace("\t", "\\t") + "\""
    }
    return """{"jsonrpc":"2.0","id":"fetchx","method":"$method","params":$paramsJson}"""
}

internal fun callAria2(config: Aria2RemoteConfig, method: String, params: List<String> = emptyList()): JSONObject {
    val connection = URL(normalizeRpcUrl(config.url)).openConnection() as HttpURLConnection
    connection.requestMethod = "POST"
    connection.connectTimeout = 15_000
    connection.readTimeout = 15_000
    connection.doOutput = true
    connection.setRequestProperty("Content-Type", "application/json; charset=utf-8")
    connection.outputStream.use { output ->
        output.write(aria2RpcRequest(method, config.secret, params).toByteArray(Charsets.UTF_8))
    }

    val responseCode = connection.responseCode
    val stream = if (responseCode in 200..299) connection.inputStream else connection.errorStream
    val body = stream?.bufferedReader()?.use(BufferedReader::readText) ?: ""
    connection.disconnect()
    if (responseCode !in 200..299) {
        throw IllegalStateException("aria2 RPC returned HTTP $responseCode")
    }
    val response = JSONObject(body)
    val error = response.optJSONObject("error")
    if (error != null) {
        throw IllegalStateException(error.optString("message", "aria2 RPC request failed"))
    }
    return response
}
