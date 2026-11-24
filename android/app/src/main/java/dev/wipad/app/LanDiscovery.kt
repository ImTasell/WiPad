package dev.wipad.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import java.io.Closeable
import java.net.InetAddress
import java.net.ServerSocket
import java.util.UUID

/**
 * First-stage LAN discovery. Advertisements are public metadata only:
 * finding a peer never authenticates it or starts an input session.
 */
class LanDiscovery(
    context: Context,
    private val onStatus: (String) -> Unit,
    private val onDesktopResolved: (InetAddress, Int, String) -> Unit = { _, _, _ -> },
) : Closeable {
    companion object {
        private const val PHONE_SERVICE = "_wipad-phone._tcp."
        private const val DESKTOP_SERVICE = "_wipad-desktop._tcp."
        private const val PROTOCOL_MAJOR = "1"
    }

    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private val wifi = context.applicationContext
        .getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val multicastLock = wifi.createMulticastLock("wipad-mdns").apply {
        setReferenceCounted(false)
    }
    private val installationId = context.getSharedPreferences("wipad", Context.MODE_PRIVATE)
        .let { prefs ->
            prefs.getString("installation_id", null) ?: UUID.randomUUID().toString().also {
                prefs.edit().putString("installation_id", it).apply()
            }
        }

    private var server: ServerSocket? = null
    private var registration: NsdManager.RegistrationListener? = null
    private var discovery: NsdManager.DiscoveryListener? = null
    private var closed = false
    private var browserRunning = false
    private var restartRequested = false
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private val resolvingServices = mutableSetOf<String>()
    private val resolvedServices = mutableSetOf<String>()

    fun start() {
        if (closed || server != null) return
        try {
            // Android 11 requires a multicast lock for reliable mDNS reception.
            multicastLock.acquire()
            val listener = ServerSocket(0)
            server = listener
            Thread({
                while (!listener.isClosed) {
                    try {
                        listener.accept().close()
                    } catch (_: Exception) {
                        if (!listener.isClosed) Thread.sleep(50)
                    }
                }
            }, "wipad-discovery-endpoint").apply {
                isDaemon = true
                start()
            }

            val phone = NsdServiceInfo().apply {
                serviceName = "WiPadPhone-${installationId.take(8)}"
                serviceType = PHONE_SERVICE
                port = listener.localPort
                setAttribute("pv", PROTOCOL_MAJOR)
                setAttribute("id", installationId)
                setAttribute("role", "phone")
                setAttribute("name", android.os.Build.MODEL.take(60))
            }
            val registrationListener = object : NsdManager.RegistrationListener {
                override fun onServiceRegistered(info: NsdServiceInfo) {
                    onStatus("Broadcasting · scanning for desktop")
                }
                override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) {
                    onStatus("Discovery advertisement failed ($errorCode)")
                }
                override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
                override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) {
                    onStatus("Could not stop discovery advertisement ($errorCode)")
                }
            }
            registration = registrationListener
            nsd.registerService(phone, NsdManager.PROTOCOL_DNS_SD, registrationListener)

            val discoveryListener = object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(serviceType: String) {
                    browserRunning = true
                    onStatus("Broadcasting · scanning for desktop")
                }
                override fun onServiceFound(service: NsdServiceInfo) {
                    if (closed || service.serviceType != DESKTOP_SERVICE) return
                    val serviceKey = service.serviceName
                    synchronized(resolvingServices) {
                        if (serviceKey in resolvingServices || serviceKey in resolvedServices) return
                        resolvingServices.add(serviceKey)
                    }
                    try {
                            nsd.resolveService(service, object : NsdManager.ResolveListener {
                                override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                                    synchronized(resolvingServices) { resolvingServices.remove(serviceKey) }
                                    if (!closed) onStatus("Found desktop · resolving failed ($errorCode)")
                                }
                                override fun onServiceResolved(info: NsdServiceInfo) {
                                    synchronized(resolvingServices) {
                                        resolvingServices.remove(serviceKey)
                                        if (closed) return
                                        resolvedServices.add(serviceKey)
                                    }
                                    val host = info.host
                                    val port = info.port
                                    if (host == null || port !in 1..65535) {
                                        onStatus("Desktop found · endpoint is invalid")
                                        return
                                    }

                                    onStatus("Desktop found · ${info.serviceName} · ${host.hostAddress}")
                                    onDesktopResolved(host, port, info.serviceName)
                                }
                            })
                    } catch (_: IllegalArgumentException) {
                        synchronized(resolvingServices) { resolvingServices.remove(serviceKey) }
                    }
                }
                override fun onServiceLost(service: NsdServiceInfo) {
                    if (service.serviceType == DESKTOP_SERVICE) {
                        synchronized(resolvingServices) {
                            resolvingServices.remove(service.serviceName)
                            resolvedServices.remove(service.serviceName)
                        }
                        if (!closed) onStatus("Desktop disappeared · scanning")
                    }
                }
                override fun onDiscoveryStopped(serviceType: String) {
                    browserRunning = false
                    if (restartRequested && !closed) {
                        restartRequested = false
                        startBrowsing()
                    }
                }
                override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                    browserRunning = false
                    restartRequested = false
                    onStatus("Discovery failed to start ($errorCode)")
                    runCatching { nsd.stopServiceDiscovery(this) }
                }
                override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                    onStatus("Could not stop discovery ($errorCode)")
                }
            }
            discovery = discoveryListener
            startBrowsing()
            val callback = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    if (!closed) {
                        onStatus("Network available · refreshing desktop discovery")
                        restartBrowser()
                    }
                }
                override fun onLost(network: Network) {
                    if (!closed) onStatus("Network changed · waiting for connectivity")
                }
            }
            networkCallback = callback
            connectivity.registerDefaultNetworkCallback(callback)
        } catch (error: Exception) {
            onStatus("LAN discovery unavailable · ${error.message ?: "unknown error"}")
            close()
        }
    }

    @Synchronized
    private fun startBrowsing() {
        if (closed || browserRunning) return
        val listener = discovery ?: return
        try {
            browserRunning = true
            nsd.discoverServices(DESKTOP_SERVICE, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (error: Exception) {
            browserRunning = false
            onStatus("Discovery restart failed · ${error.message ?: "unknown error"}")
        }
    }

    @Synchronized
    private fun restartBrowser() {
        if (closed) return
        val listener = discovery ?: return
        if (browserRunning) {
            restartRequested = true
            runCatching { nsd.stopServiceDiscovery(listener) }
                .onFailure { restartRequested = false }
        } else {
            startBrowsing()
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        networkCallback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        networkCallback = null
        synchronized(resolvingServices) {
            resolvingServices.clear()
            resolvedServices.clear()
        }
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        registration?.let { runCatching { nsd.unregisterService(it) } }
        server?.let { runCatching { it.close() } }
        server = null
        if (multicastLock.isHeld) multicastLock.release()
    }
}
