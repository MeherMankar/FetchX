package dev.fetchx.app.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.content.Intent

internal object DownloadNetworkPolicy {
    const val PREFERENCES_NAME = "fetchx_download_network_policy"
    const val WIFI_ONLY_KEY = "wifi_only"
    const val UNMETERED_ONLY_KEY = "unmetered_only"
    const val ROAMING_ALLOWED_KEY = "roaming_allowed"
    const val CHARGING_ONLY_KEY = "charging_only"
    const val BATTERY_CONTROL_KEY = "battery_control"
    const val BATTERY_THRESHOLD_KEY = "battery_threshold"

    fun isWifiOnlyEnabled(context: Context): Boolean =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .getBoolean(WIFI_ONLY_KEY, false)

    fun setWifiOnlyEnabled(context: Context, enabled: Boolean) {
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(WIFI_ONLY_KEY, enabled)
            .apply()
    }

    fun setPolicy(context: Context, key: String, enabled: Boolean) {
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(key, enabled)
            .apply()
    }

    fun setBatteryThreshold(context: Context, threshold: Int) {
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .edit()
            .putInt(BATTERY_THRESHOLD_KEY, threshold.coerceIn(0, 100))
            .apply()
    }

    fun isEnabled(context: Context, key: String): Boolean =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .getBoolean(key, key == ROAMING_ALLOWED_KEY)

    fun getBatteryThreshold(context: Context): Int =
        context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .getInt(BATTERY_THRESHOLD_KEY, 20)

    fun isDownloadAllowed(context: Context): Boolean {
        val connectivityManager =
            context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = connectivityManager.activeNetwork ?: return false
        val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return false
        val hasWifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        val hasInternet = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        val isUnmetered = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        val notRoaming = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_ROAMING)
        val batteryStatus = context.registerReceiver(
            null,
            android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED),
        )
        val batteryLevel = batteryStatus?.let { intent ->
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
            if (level >= 0 && scale > 0) level * 100 / scale else -1
        } ?: -1
        val chargingStatus = batteryStatus?.getIntExtra(
            BatteryManager.EXTRA_STATUS,
            BatteryManager.BATTERY_STATUS_UNKNOWN,
        )
        val isCharging = chargingStatus == BatteryManager.BATTERY_STATUS_CHARGING ||
            chargingStatus == BatteryManager.BATTERY_STATUS_FULL

        return isTransferAllowed(
            wifiOnly = isWifiOnlyEnabled(context),
            unmeteredOnly = isEnabled(context, UNMETERED_ONLY_KEY),
            roamingAllowed = isEnabled(context, ROAMING_ALLOWED_KEY),
            chargingOnly = isEnabled(context, CHARGING_ONLY_KEY),
            batteryControl = isEnabled(context, BATTERY_CONTROL_KEY),
            hasWifi = hasWifi,
            hasInternet = hasInternet,
            isUnmetered = isUnmetered,
            isRoaming = !notRoaming,
            isCharging = isCharging,
            batteryLevel = batteryLevel,
            batteryThreshold = getBatteryThreshold(context),
        )
    }
}

internal fun canDownload(wifiOnly: Boolean, hasWifi: Boolean): Boolean =
    !wifiOnly || hasWifi

internal fun isTransferAllowed(
    wifiOnly: Boolean,
    unmeteredOnly: Boolean,
    roamingAllowed: Boolean,
    chargingOnly: Boolean,
    batteryControl: Boolean,
    hasWifi: Boolean,
    hasInternet: Boolean,
    isUnmetered: Boolean,
    isRoaming: Boolean,
    isCharging: Boolean,
    batteryLevel: Int,
    batteryThreshold: Int,
): Boolean {
    if (!hasInternet) return false
    if (wifiOnly && !hasWifi) return false
    if (unmeteredOnly && !isUnmetered) return false
    if (isRoaming && !roamingAllowed) return false
    if (chargingOnly && !isCharging) return false
    if (batteryControl && batteryLevel >= 0 && batteryLevel < batteryThreshold) return false
    return true
}
