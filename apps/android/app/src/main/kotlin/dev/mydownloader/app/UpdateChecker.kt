package dev.fetchx.app

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.URL

data class UpdateResult(val latestVersion: String, val releaseUrl: String)

suspend fun checkForUpdate(currentVersion: String): UpdateResult? = withContext(Dispatchers.IO) {
    try {
        val connection = URL("https://api.github.com/repos/MeherMankar/FetchX/releases/latest")
            .openConnection()
        connection.setRequestProperty("User-Agent", "FetchX/$currentVersion")
        connection.connectTimeout = 10_000
        connection.readTimeout = 10_000
        val body = connection.getInputStream().bufferedReader().readText()
        val json = JSONObject(body)
        val tag = json.optString("tag_name", "").trimStart('v')
        val htmlUrl = json.optString("html_url", "")
        if (tag.isNotEmpty() && tag != currentVersion) {
            UpdateResult(latestVersion = tag, releaseUrl = htmlUrl)
        } else {
            null
        }
    } catch (e: Exception) {
        Log.w("UpdateChecker", "Update check failed: $e")
        null
    }
}
