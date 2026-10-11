package dev.mydownloader.core.network

/**
 * The network routing mode for a download or browser session.
 * Implemented in Phase 20M (Tor/VPN architecture).
 */
sealed class NetworkRoute {
    /** Use the device's default network interface. */
    data object Direct : NetworkRoute()

    /** Route through the system VPN (device-level VPN app, e.g. Mullvad, ProtonVPN). */
    data object SystemVpn : NetworkRoute()

    /** Route through an explicit SOCKS5 proxy. */
    data class SocksProxy(val host: String, val port: Int) : NetworkRoute()

    /**
     * Route through Tor via the Orbot SOCKS5 interface (127.0.0.1:9050 default).
     * Requires Orbot or a bundled Tor binary (Phase 20M decision).
     */
    data object Tor : NetworkRoute()
}
