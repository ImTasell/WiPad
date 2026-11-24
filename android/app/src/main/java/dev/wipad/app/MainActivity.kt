package dev.wipad.app

import android.os.Bundle
import java.net.InetAddress
import java.util.concurrent.atomic.AtomicBoolean
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

private val WiPadColors = darkColorScheme(
    primary = Color(0xFF64B5F6),
    secondary = Color(0xFF4DD0E1),
    background = Color(0xFF0D1B2A),
    surface = Color(0xFF14283B),
)

class MainActivity : ComponentActivity() {
    private var desktopStatus by mutableStateOf("Starting LAN discovery…")
    private var discovery: LanDiscovery? = null
    private val sessionConnecting = AtomicBoolean(false)
    @Volatile private var activityStarted = false
    @Volatile private var phoneSession: PhoneSession? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(colorScheme = WiPadColors) {
                WiPadHome(desktopStatus)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        activityStarted = true
        discovery = LanDiscovery(
            context = this,
            onStatus = { status ->
                runOnUiThread { desktopStatus = status }
            },
            onDesktopResolved = { address, port, serviceName ->
                connectToDesktop(address, port, serviceName)
            },
        ).also { it.start() }
    }

    override fun onStop() {
        activityStarted = false
        discovery?.close()
        discovery = null
        phoneSession?.close()
        phoneSession = null
        super.onStop()
    }

    private fun connectToDesktop(address: InetAddress, port: Int, serviceName: String) {
        if (phoneSession != null || !sessionConnecting.compareAndSet(false, true)) return
        runOnUiThread { desktopStatus = "Securing connection to $serviceName" }

        Thread({
            try {
                val identity = TlsIdentity.loadOrCreate()
                val pinStore = TlsPeerPinStore(this)
                val session = TlsPairingClient(identity, pinStore).connect(
                    address = address,
                    port = port,
                    displayName = "WiPad Phone",
                ) { request ->
                    runOnUiThread {
                        desktopStatus = "Pairing needs approval · ${request.peerDisplayName} · ${request.comparisonString}"
                    }
                    // The approval surface is not implemented yet. Never auto-approve first contact.
                    false
                }

                if (activityStarted) {
                    phoneSession = session
                    runOnUiThread {
                        desktopStatus = "Secure session ready · ${session.peerDisplayName}"
                    }
                } else {
                    session.close()
                }
            } catch (error: Exception) {
                if (activityStarted) {
                    runOnUiThread {
                        desktopStatus = "Secure session unavailable · ${error.message ?: error.javaClass.simpleName}"
                    }
                }
            } finally {
                sessionConnecting.set(false)
            }
        }, "wipad-tls-session").apply {
            isDaemon = true
            start()
        }
    }
}

@androidx.compose.runtime.Composable
private fun WiPadHome(desktopStatus: String) {
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("WiPad", style = MaterialTheme.typography.headlineLarge)
            Spacer(Modifier.height(12.dp))
            Text(
                "Your phone is ready to become a controller bridge.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onBackground,
            )
            Spacer(Modifier.height(28.dp))
            Text(
                "Bluetooth controller: Not connected",
                color = MaterialTheme.colorScheme.secondary,
            )
            Text(
                "Desktop: $desktopStatus",
                color = MaterialTheme.colorScheme.onBackground,
            )
        }
    }
}
