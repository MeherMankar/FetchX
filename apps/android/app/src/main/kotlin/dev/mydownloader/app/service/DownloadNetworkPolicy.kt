package dev.fetchx.app.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities

internal object DownloadNetworkPolicy {
    const val PREFERENCES_NAME = "fetchx_download_network_policy"
    const val WIFI_ONLY_KEY = "wifi_only"

    fun isWifiOnlyEnabled(context: Context): Boolean =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .getBoolean(WIFI_ONLY_KEY, false)

    fun setWifiOnlyEnabled(context: Context, enabled: Boolean) {
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(WIFI_ONLY_KEY, enabled)
            .apply()
    }

    fun isDownloadAllowed(context: Context): Boolean {
        if (!isWifiOnlyEnabled(context)) return true
        val connectivityManager =
            context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = connectivityManager.activeNetwork ?: return false
        val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return false
        val hasWifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        return canDownload(wifiOnly = true, hasWifi = hasWifi)
    }
}

internal fun canDownload(wifiOnly: Boolean, hasWifi: Boolean): Boolean =
    !wifiOnly || hasWifi
