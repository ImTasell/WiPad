package dev.wipad.app

import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import java.net.InetAddress
import javax.net.ssl.SSLSocket
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.ThreadLocalRandom
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

private data class DesktopEndpoint(
    val address: InetAddress,
    val port: Int,
    val serviceName: String,
)

class MainActivity : ComponentActivity() {
    private var desktopStatus by mutableStateOf("Starting LAN discovery…")
    private var pendingPairing by mutableStateOf<PairingPrompt?>(null)
    private val pairingApprovals = PairingApprovalCoordinator(
        onPendingChanged = { prompt -> runOnUiThread { pendingPairing = prompt } },
    )
    private var discovery: LanDiscovery? = null
    @Volatile private var discoveredDesktop: DesktopEndpoint? = null
    private val sessionConnecting = AtomicBoolean(false)
    private val trustRevisionLock = Any()
    private var trustRevision = 0L
    private val reconnectAfterTrustRevocation = AtomicBoolean(false)
    private val controllerInput = AndroidControllerInput()
    private val inputSequence = AtomicLong(0)
    @Volatile private var activityStarted = false
    @Volatile private var phoneSession: PhoneSession? = null
    @Volatile private var connectingSocket: SSLSocket? = null
    @Volatile private var inputThread: Thread? = null
    @Volatile private var heartbeatThread: Thread? = null
    @Volatile private var connectionThread: Thread? = null

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
            onDesktopLost = ::onDesktopLost,
        ).also { it.start() }
    }

    override fun onStop() {
        activityStarted = false
        discoveredDesktop = null
        pairingApprovals.cancelPending()
        discovery?.close()
        discovery = null
        val activeSession = phoneSession
        phoneSession = null
        runCatching { connectingSocket?.close() }
        connectingSocket = null
        runCatching { activeSession?.close() }

        connectionThread?.interrupt()
        inputThread?.interrupt()
        heartbeatThread?.interrupt()
        joinWorker(inputThread)
        joinWorker(heartbeatThread)
        joinWorker(connectionThread)
        connectionThread = null
        inputThread = null
        heartbeatThread = null
        super.onStop()
    }

    /** Approval surface hook: UI code can accept or reject the pending first-contact peer. */
    fun approvePendingPairing(): Boolean = pairingApprovals.decide(true)

    fun rejectPendingPairing(): Boolean = pairingApprovals.decide(false)

    internal fun currentPairingPrompt(): PairingPrompt? = pairingApprovals.pendingRequest()

    fun forgetTrustedDesktop(): Boolean {
        pairingApprovals.cancelPending()
        val sockets = synchronized(trustRevisionLock) {
            trustRevision += 1
            val active = phoneSession
            phoneSession = null
            val connecting = connectingSocket
            connectingSocket = null
            active to connecting
        }
        runCatching { sockets.first?.close() }
        runCatching { sockets.second?.close() }

        val revoked = runCatching { TlsPeerPinStore(this).forgetDesktop() }.isSuccess
        if (!revoked) {
            runOnUiThread { desktopStatus = "Could not revoke trusted desktop" }
            return false
        }

        runOnUiThread { desktopStatus = "Trusted desktop forgotten · requesting approval" }
        if (sessionConnecting.get()) {
            reconnectAfterTrustRevocation.set(true)
        } else {
            discoveredDesktop?.let { endpoint ->
                connectToDesktop(endpoint.address, endpoint.port, endpoint.serviceName)
            }
        }
        return true
    }

    private fun joinWorker(worker: Thread?) {
        if (worker == null || worker === Thread.currentThread()) return
        try {
            worker.join(750)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    private fun isTerminalPairingFailure(error: Throwable): Boolean =
        generateSequence(error) { it.cause }.any { cause ->
            val causeMessage = cause.message.orEmpty()
            listOf(
                "rejected",
                "identity changed",
                "certificate changed",
                "incompatible",
                "unsupported",
                "trusted identity",
            ).any { marker -> causeMessage.contains(marker, ignoreCase = true) }
        }

    private fun onDesktopLost(serviceName: String) {
        if (discoveredDesktop?.serviceName != serviceName) return
        discoveredDesktop = null
        phoneSession?.let { session ->
            phoneSession = null
            runCatching { session.close() }
        }
    }

    private fun connectToDesktop(address: InetAddress, port: Int, serviceName: String) {
        discoveredDesktop = DesktopEndpoint(address, port, serviceName)
        if (phoneSession != null || !sessionConnecting.compareAndSet(false, true)) return
        runOnUiThread { desktopStatus = "Securing connection to $serviceName" }

        val worker = Thread({
            var attempt = 0
            try {
                while (activityStarted && !Thread.currentThread().isInterrupted) {
                    val endpoint = discoveredDesktop
                    if (endpoint == null) {
                        Thread.sleep(250)
                        continue
                    }
                    try {
                        val attemptTrustRevision = synchronized(trustRevisionLock) { trustRevision }
                        val identity = TlsIdentity.loadOrCreate()
                        val pinStore = TlsPeerPinStore(this)
                        val session = TlsPairingClient(identity, pinStore).connect(
                            address = endpoint.address,
                            port = endpoint.port,
                            displayName = "WiPad Phone",
                            onSocketCreated = { socket ->
                                val keepSocket = synchronized(trustRevisionLock) {
                                    if (activityStarted && attemptTrustRevision == trustRevision) {
                                        connectingSocket = socket
                                        true
                                    } else {
                                        false
                                    }
                                }
                                if (!keepSocket) runCatching { socket.close() }
                            },
                        ) { request ->
                            pairingApprovals.requestApproval(request)
                        }

                        val accepted = synchronized(trustRevisionLock) {
                            if (
                                attemptTrustRevision == trustRevision
                                && activityStarted
                                && !Thread.currentThread().isInterrupted
                                && discoveredDesktop == endpoint
                            ) {
                                phoneSession = session
                                connectingSocket = null
                                true
                            } else {
                                false
                            }
                        }
                        if (!accepted) {
                            if (attemptTrustRevision != synchronized(trustRevisionLock) { trustRevision }) {
                                runCatching { pinStore.forgetDesktop() }
                            }
                            connectingSocket = null
                            session.close()
                            if (!activityStarted || Thread.currentThread().isInterrupted) break
                            continue
                        }
                        reconnectAfterTrustRevocation.set(false)
                        attempt = 0
                        runOnUiThread {
                            desktopStatus = "Secure session ready · ${session.peerDisplayName}"
                        }
                        startHeartbeatResponder(session)
                        startControllerStream(session)

                        while (
                            activityStarted
                            && phoneSession === session
                            && !Thread.currentThread().isInterrupted
                        ) {
                            Thread.sleep(100)
                        }
                        if (activityStarted && !Thread.currentThread().isInterrupted) {
                            runOnUiThread { desktopStatus = "Connection lost · reconnecting" }
                        }
                    } catch (error: Exception) {
                        if (!activityStarted || Thread.currentThread().isInterrupted) break
                        val message = error.message ?: error.javaClass.simpleName
                        runOnUiThread { desktopStatus = "Secure session unavailable · $message" }

                        if (isTerminalPairingFailure(error)) break
                    }

                    if (activityStarted && !Thread.currentThread().isInterrupted) {
                        val jitter = ThreadLocalRandom.current().nextLong(0L, 501L)
                        val delay = ReconnectBackoff.delayMillis(attempt, jitter)
                        attempt = (attempt + 1).coerceAtMost(5)
                        Thread.sleep(delay)
                    }
                }
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            } finally {
                connectingSocket = null
                sessionConnecting.set(false)
                if (reconnectAfterTrustRevocation.compareAndSet(true, false) && activityStarted) {
                    discoveredDesktop?.let { endpoint ->
                        connectToDesktop(endpoint.address, endpoint.port, endpoint.serviceName)
                    }
                }
            }
        }, "wipad-connection-manager").apply {
            isDaemon = true
            start()
        }
        connectionThread = worker
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        controllerInput.refreshDevices()
        return if (controllerInput.handleKeyEvent(event)) true else super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        controllerInput.refreshDevices()
        return if (controllerInput.handleKeyEvent(event)) true else super.onKeyUp(keyCode, event)
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        controllerInput.refreshDevices()
        return if (controllerInput.handleMotionEvent(event)) true else super.onGenericMotionEvent(event)
    }

    private fun startControllerStream(session: PhoneSession) {
        inputThread?.interrupt()
        val worker = Thread({
            var sequence = inputSequence.get()
            var lastControllerConnected: Boolean? = null
            try {
                while (
                    activityStarted
                    && phoneSession === session
                    && !Thread.currentThread().isInterrupted
                ) {
                    controllerInput.refreshDevices()
                    val controllerConnected = controllerInput.isConnected()
                    if (lastControllerConnected != controllerConnected) {
                        session.sendControllerStatus(controllerConnected)
                        lastControllerConnected = controllerConnected
                        runOnUiThread {
                            desktopStatus = if (controllerConnected) {
                                "Controller connected · streaming input"
                            } else {
                                "No controller connected · waiting for Bluetooth gamepad"
                            }
                        }
                    }
                    sequence += 1
                    val snapshot = controllerInput.snapshot(
                        sequence = sequence,
                        sampleTimeMicros = System.nanoTime() / 1_000L,
                    )
                    session.sendControllerState(snapshot)
                    inputSequence.set(sequence)
                    Thread.sleep(INPUT_INTERVAL_MILLIS)
                }
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            } catch (error: Exception) {
                if (phoneSession === session) {
                    phoneSession = null
                    session.close()
                    if (activityStarted) {
                        runOnUiThread {
                            desktopStatus = "Controller stream ended · ${error.message ?: error.javaClass.simpleName}"
                        }
                    }
                }
            }
        }, "wipad-controller-stream").apply {
            isDaemon = true
            start()
        }
        inputThread = worker
    }

    private fun startHeartbeatResponder(session: PhoneSession) {
        heartbeatThread?.interrupt()
        val worker = Thread({
            try {
                session.respondToHeartbeats()
            } catch (error: Exception) {
                if (phoneSession === session) {
                    phoneSession = null
                    session.close()
                    if (activityStarted) {
                        runOnUiThread {
                            desktopStatus = "Desktop heartbeat lost · reconnecting"
                        }
                    }
                }
            }
        }, "wipad-session-heartbeat").apply {
            isDaemon = true
            start()
        }
        heartbeatThread = worker
    }

    companion object {
        private const val INPUT_INTERVAL_MILLIS = 16L
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
