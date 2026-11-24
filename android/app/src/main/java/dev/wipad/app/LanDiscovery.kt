package dev.wipad.app

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import java.io.Closeable
import java.net.ServerSocket
import java.util.UUID

/**
 * First-stage LAN discovery. Advertisements are public metadata only:
 * finding a peer never authenticates it or starts an input session.
 */
class LanDiscovery(
    context: Context,
    private val onStatus: (String) -> Unit,
) : Closeable {
    companion object {
        private const val PHONE_SERVICE = "_wipad-phone._tcp."
        private const val DESKTOP_SERVICE = "_wipad-desktop._tcp."
        private const val PROTOCOL_MAJOR = "1"
    }

    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
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
                serviceName = "WiPad Phone"
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
                    onStatus("Broadcasting · scanning for desktop")
                }
                override fun onServiceFound(service: NsdServiceInfo) {
                    if (service.serviceType == DESKTOP_SERVICE) {
                        try {
                            nsd.resolveService(service, object : NsdManager.ResolveListener {
                                override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                                    onStatus("Found desktop · resolving failed ($errorCode)")
                                }
                                override fun onServiceResolved(info: NsdServiceInfo) {
                                    val address = info.host?.hostAddress ?: "address unavailable"
                                    onStatus("Desktop found · ${info.serviceName} · $address")
                                }
                            })
                        } catch (_: IllegalArgumentException) {
                            // A duplicate resolution may already be in progress.
                        }
                    }
                }
                override fun onServiceLost(service: NsdServiceInfo) {
                    if (service.serviceType == DESKTOP_SERVICE) {
                        onStatus("Desktop disappeared · scanning")
                    }
                }
                override fun onDiscoveryStopped(serviceType: String) = Unit
                override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                    onStatus("Discovery failed to start ($errorCode)")
                    runCatching { nsd.stopServiceDiscovery(this) }
                }
                override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                    onStatus("Could not stop discovery ($errorCode)")
                }
            }
            discovery = discoveryListener
            nsd.discoverServices(DESKTOP_SERVICE, NsdManager.PROTOCOL_DNS_SD, discoveryListener)
        } catch (error: Exception) {
            onStatus("LAN discovery unavailable · ${error.message ?: "unknown error"}")
            close()
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        registration?.let { runCatching { nsd.unregisterService(it) } }
        server?.let { runCatching { it.close() } }
        server = null
        if (multicastLock.isHeld) multicastLock.release()
    }
}
