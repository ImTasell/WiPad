package dev.wipad.app

import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.security.cert.X509Certificate
import javax.net.ssl.SSLSocket

private const val HEARTBEAT_TIMEOUT_MILLIS = 7_000

internal data class PairingPrompt(
    val peerDisplayName: String,
    val comparisonString: String,
    val peerFingerprint: ByteArray,
    val reconnecting: Boolean,
)

/** An authenticated TLS connection that has completed both approval decisions. */
internal class PhoneSession(
    val socket: SSLSocket,
    val peerDisplayName: String,
    val comparisonString: String,
    val peerFingerprint: ByteArray,
    val reconnecting: Boolean,
) : Closeable {
    @Synchronized
    fun sendControllerState(state: ControllerState) {
        if (socket.isClosed || socket.isOutputShutdown) {
            throw IOException("WiPad session is closed")
        }
        FrameCodec.writeFrame(
            socket.outputStream,
            ApplicationMessageCodec.encode(ApplicationMessage.InputState(state)),
        )
    }

    @Synchronized
    fun sendControllerStatus(connected: Boolean) {
        if (socket.isClosed || socket.isOutputShutdown) {
            throw IOException("WiPad session is closed")
        }
        FrameCodec.writeFrame(
            socket.outputStream,
            ApplicationMessageCodec.encode(ApplicationMessage.ControllerStatus(connected)),
        )
    }

    @Synchronized
    fun sendHeartbeatResponse(nonce: Long) {
        if (socket.isClosed || socket.isOutputShutdown) {
            throw IOException("WiPad session is closed")
        }
        FrameCodec.writeFrame(
            socket.outputStream,
            ApplicationMessageCodec.encode(ApplicationMessage.Pong(nonce)),
        )
    }

    /**
     * Owns the session read direction. The desktop sends periodic PING frames;
     * a timeout or unexpected message is a dead/invalid session.
     */
    fun respondToHeartbeats() {
        socket.soTimeout = HEARTBEAT_TIMEOUT_MILLIS
        while (!socket.isClosed && !Thread.currentThread().isInterrupted) {
            val message = ApplicationMessageCodec.decode(FrameCodec.readFrame(socket.inputStream))
            when (message) {
                is ApplicationMessage.Ping -> {
                    sendHeartbeatResponse(message.nonce)
                }
                else -> throw IOException("Desktop sent an invalid heartbeat-session message")
            }
        }
    }

    @Synchronized
    override fun close() = socket.close()
}

/**
 * Connects to the discovered desktop and completes the two-sided pairing protocol.
 *
 * A first-contact certificate is pinned only after both devices have approved.
 * A matching stored pin auto-approves this phone's side, but the desktop must
 * still send its own approval decision on every connection.
 */
internal class TlsPairingClient(
    private val identity: TlsIdentity,
    private val pinStore: TlsPeerPinStore,
) {
    @Throws(Exception::class)
    fun connect(
        address: InetAddress,
        port: Int,
        displayName: String,
        onSocketCreated: (SSLSocket) -> Unit = {},
        approveLocal: (PairingPrompt) -> Boolean,
    ): PhoneSession {
        val storedPin = pinStore.loadDesktopFingerprint()
        var socket: SSLSocket? = null

        try {
            socket = identity.connect(address, port, storedPin, onSocketCreated = onSocketCreated)
            socket.soTimeout = PAIRING_READ_TIMEOUT_MILLIS

            val peerCertificate = socket.session.peerCertificates
                .firstOrNull() as? X509Certificate
                ?: throw IOException("Desktop did not present an X.509 identity certificate")

            val localCertificate = identity.certificate
            val peerFingerprint = TlsFingerprint.certificateFingerprint(peerCertificate.encoded)
            val reconnecting = storedPin != null

            if (storedPin != null
                && !TlsFingerprint.matchesFingerprint(storedPin, peerFingerprint)
            ) {
                throw IOException("Desktop identity changed; pairing approval is required")
            }

            writeMessage(
                socket,
                ApplicationMessage.PairingHello(
                    major = ApplicationMessageCodec.PROTOCOL_MAJOR,
                    minor = ApplicationMessageCodec.PROTOCOL_MINOR,
                    role = PeerRole.PHONE,
                    displayName = displayName,
                ),
            )

            val peerHello = readMessage(socket) as? ApplicationMessage.PairingHello
                ?: throw IOException("Desktop did not send a WiPad pairing hello")
            if (peerHello.major != ApplicationMessageCodec.PROTOCOL_MAJOR
                || peerHello.minor < ApplicationMessageCodec.PROTOCOL_MINOR
                || peerHello.role != PeerRole.DESKTOP
            ) {
                throw IOException("Desktop sent an incompatible WiPad pairing hello")
            }

            val comparisonString = TlsFingerprint.pairingString(
                localCertificate.encoded,
                peerCertificate.encoded,
            )
            val gate = PairingGate(localPinMatches = reconnecting)
            val prompt = PairingPrompt(
                peerDisplayName = peerHello.displayName,
                comparisonString = comparisonString,
                peerFingerprint = peerFingerprint.copyOf(),
                reconnecting = reconnecting,
            )

            val localApproved = reconnecting || approveLocal(prompt)
            writeMessage(socket, ApplicationMessage.PairingDecision(localApproved))
            if (!localApproved) {
                throw IOException("Local pairing approval was rejected")
            }
            if (!reconnecting) gate.approveLocal()

            val peerDecision = readMessage(socket) as? ApplicationMessage.PairingDecision
                ?: throw IOException("Desktop did not send a WiPad pairing decision")
            gate.receivePeerDecision(peerDecision.approved)
            if (!gate.canStorePeerPin()) {
                throw IOException("Desktop rejected WiPad pairing")
            }

            writeMessage(socket, ApplicationMessage.SessionReady)
            if (readMessage(socket) != ApplicationMessage.SessionReady) {
                throw IOException("Desktop did not confirm SESSION_READY")
            }

            // Persist only after both approvals and both SESSION_READY messages.
            if (!reconnecting) pinStore.approveDesktop(peerCertificate)

            return PhoneSession(
                socket = socket,
                peerDisplayName = peerHello.displayName,
                comparisonString = comparisonString,
                peerFingerprint = peerFingerprint,
                reconnecting = reconnecting,
            )
        } catch (error: Exception) {
            runCatching { socket?.close() }
            throw error
        }
    }

    private fun writeMessage(socket: SSLSocket, message: ApplicationMessage) {
        FrameCodec.writeFrame(socket.outputStream, ApplicationMessageCodec.encode(message))
    }

    private fun readMessage(socket: SSLSocket): ApplicationMessage =
        ApplicationMessageCodec.decode(FrameCodec.readFrame(socket.inputStream))

    companion object {
        private const val PAIRING_READ_TIMEOUT_MILLIS = 120_000
    }
}
