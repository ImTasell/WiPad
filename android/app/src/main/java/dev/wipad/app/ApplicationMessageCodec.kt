package dev.wipad.app

import java.io.IOException
import java.nio.charset.StandardCharsets

internal enum class PeerRole(val wireValue: Int) {
    PHONE(1),
    DESKTOP(2);

    companion object {
        fun fromWire(value: Int): PeerRole =
            entries.firstOrNull { it.wireValue == value }
                ?: throw IOException("WiPad peer role is invalid")
    }
}

internal sealed class ApplicationMessage {
    data class PairingHello(
        val major: Int,
        val minor: Int,
        val role: PeerRole,
        val displayName: String,
    ) : ApplicationMessage()

    data class PairingDecision(val approved: Boolean) : ApplicationMessage()

    data object SessionReady : ApplicationMessage()

    data class InputState(val state: ControllerState) : ApplicationMessage()
}

/**
 * Language-neutral WiPad application messages.
 *
 * These payloads are written as WiPad frames only after TLS 1.3 has completed.
 */
internal object ApplicationMessageCodec {
    const val PROTOCOL_MAJOR = 1
    const val PROTOCOL_MINOR = 0
    const val MAX_DISPLAY_NAME_LENGTH = 64

    private const val MESSAGE_PAIRING_HELLO = 0x01
    private const val MESSAGE_PAIRING_DECISION = 0x02
    private const val MESSAGE_SESSION_READY = 0x03
    private const val MESSAGE_INPUT_STATE = ControllerStateCodec.MESSAGE_TYPE

    fun encode(message: ApplicationMessage): ByteArray {
        val payload = when (message) {
            is ApplicationMessage.PairingHello -> {
                val name = message.displayName.toByteArray(StandardCharsets.UTF_8)
                require(message.major == PROTOCOL_MAJOR)
                require(name.isNotEmpty() && name.size <= MAX_DISPLAY_NAME_LENGTH)
                require(message.displayName.none(Char::isISOControl))

                byteArrayOf(
                    MESSAGE_PAIRING_HELLO.toByte(),
                    message.major.toByte(),
                    message.minor.toByte(),
                    message.role.wireValue.toByte(),
                    name.size.toByte(),
                ) + name
            }

            is ApplicationMessage.PairingDecision -> byteArrayOf(
                MESSAGE_PAIRING_DECISION.toByte(),
                if (message.approved) 1 else 0,
            )

            ApplicationMessage.SessionReady -> byteArrayOf(MESSAGE_SESSION_READY.toByte())

            is ApplicationMessage.InputState -> ControllerStateCodec.encode(message.state)
        }

        if (payload.isEmpty() || payload.size > FrameCodec.MAX_FRAME_LENGTH) {
            throw IOException("WiPad application message length is invalid")
        }
        return payload
    }

    @Throws(IOException::class)
    fun decode(payload: ByteArray): ApplicationMessage {
        if (payload.isEmpty() || payload.size > FrameCodec.MAX_FRAME_LENGTH) {
            throw IOException("WiPad application message length is invalid")
        }

        return when (payload[0].toInt() and 0xFF) {
            MESSAGE_PAIRING_HELLO -> decodePairingHello(payload)
            MESSAGE_INPUT_STATE -> ApplicationMessage.InputState(ControllerStateCodec.decode(payload))
            MESSAGE_PAIRING_DECISION -> {
                if (payload.size != 2 || (payload[1].toInt() and 0xFF) !in 0..1) {
                    throw IOException("WiPad pairing decision is malformed")
                }
                ApplicationMessage.PairingDecision(payload[1].toInt() == 1)
            }

            MESSAGE_SESSION_READY -> {
                if (payload.size != 1) {
                    throw IOException("WiPad SESSION_READY must not have a payload")
                }
                ApplicationMessage.SessionReady
            }

            else -> throw IOException("WiPad application message type is unknown")
        }
    }

    private fun decodePairingHello(payload: ByteArray): ApplicationMessage.PairingHello {
        if (payload.size < 6) throw IOException("WiPad pairing hello is truncated")

        val major = payload[1].toInt() and 0xFF
        val minor = payload[2].toInt() and 0xFF
        if (major != PROTOCOL_MAJOR) {
            throw IOException("WiPad protocol major version is unsupported")
        }

        val role = PeerRole.fromWire(payload[3].toInt() and 0xFF)
        val nameLength = payload[4].toInt() and 0xFF
        if (nameLength !in 1..MAX_DISPLAY_NAME_LENGTH || payload.size != 5 + nameLength) {
            throw IOException("WiPad pairing hello display-name length is invalid")
        }

        val name = payload.copyOfRange(5, payload.size).toString(StandardCharsets.UTF_8)
        if (name.any(Char::isISOControl)) {
            throw IOException("WiPad display name contains control characters")
        }

        return ApplicationMessage.PairingHello(major, minor, role, name)
    }
}
