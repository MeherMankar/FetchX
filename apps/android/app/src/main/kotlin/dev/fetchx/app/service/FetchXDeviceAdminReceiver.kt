package dev.fetchx.app.service

import android.app.admin.DeviceAdminReceiver

/**
 * Minimal DeviceAdminReceiver that enables FetchX to call
 * [android.app.admin.DevicePolicyManager.lockNow] when the download
 * queue goes idle and the user has selected "Turn off screen" in settings.
 *
 * The app must be explicitly activated as a device admin by the user via
 * [android.app.admin.DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN] before
 * lockNow() is callable.
 */
class FetchXDeviceAdminReceiver : DeviceAdminReceiver()
