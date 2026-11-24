package dev.wipad.app

import java.io.IOException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class ApplicationMessageCodecTest {
    @Test
    fun encodesAndDecodesBothPeerRolesAgainstSharedVectors() {
        val cases = listOf(
            ApplicationMessage.PairingHello(1, 2, PeerRole.PHONE, "Phone") to "pairing_hello_phone",
            ApplicationMessage.PairingHello(1, 2, PeerRole.DESKTOP, "Linux") to "pairing_hello_desktop",
            ApplicationMessage.PairingHello(1, 2, PeerRole.PHONE, "Phone 📱") to "pairing_hello_phone_unicode",
        )

        cases.forEach { (message, vectorName) ->
            val vector = GoldenVectors.bytes(vectorName)
            assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
            assertEquals(message, ApplicationMessageCodec.decode(vector))
        }
    }

    @Test
    fun encodesAndDecodesBothPairingDecisionsAndSessionReady() {
        val cases = listOf(
            ApplicationMessage.PairingDecision(false) to "pairing_decision_rejected",
            ApplicationMessage.PairingDecision(true) to "pairing_decision_approved",
        )

        cases.forEach { (message, vectorName) ->
            val vector = GoldenVectors.bytes(vectorName)
            assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
            assertEquals(message, ApplicationMessageCodec.decode(vector))
        }

        val ready = GoldenVectors.bytes("session_ready")
        assertArrayEquals(ready, ApplicationMessageCodec.encode(ApplicationMessage.SessionReady))
        assertEquals(ApplicationMessage.SessionReady, ApplicationMessageCodec.decode(ready))
    }

    @Test
    fun encodesAndDecodesControllerPresenceAgainstSharedVectors() {
        val cases = listOf(
            ApplicationMessage.ControllerStatus(false) to "controller_status_disconnected",
            ApplicationMessage.ControllerStatus(true) to "controller_status_connected",
        )

        cases.forEach { (message, vectorName) ->
            val vector = GoldenVectors.bytes(vectorName)
            assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
            assertEquals(message, ApplicationMessageCodec.decode(vector))
        }
    }

    @Test
    fun encodesAndDecodesHeartbeatNoncesAgainstSharedVectors() {
        val ping = ApplicationMessage.Ping(0x0102030405060708L)
        val pong = ApplicationMessage.Pong(0x0102030405060708L)
        val pingVector = GoldenVectors.bytes("ping_nonce_0102030405060708")
        val pongVector = GoldenVectors.bytes("pong_nonce_0102030405060708")

        assertArrayEquals(pingVector, ApplicationMessageCodec.encode(ping))
        assertEquals(ping, ApplicationMessageCodec.decode(pingVector))
        assertArrayEquals(pongVector, ApplicationMessageCodec.encode(pong))
        assertEquals(pong, ApplicationMessageCodec.decode(pongVector))
    }

    @Test
    fun rejectsSharedMalformedMessageVectors() {
        listOf(
            "invalid_unknown_message",
            "invalid_pairing_hello_major",
            "invalid_pairing_hello_role",
            "invalid_pairing_hello_utf8",
            "invalid_pairing_hello_control",
            "invalid_pairing_decision_boolean",
            "invalid_controller_status_boolean",
            "invalid_ping_truncated",
            "invalid_pong_extra_byte",
            "invalid_input_state_hat",
        ).forEach { vectorName ->
            assertDecodeRejected(GoldenVectors.bytes(vectorName))
        }
    }

    @Test
    fun rejectsInvalidDisplayNamesAndOutOfRangeMinorVersions() {
        val tooLong = ApplicationMessage.PairingHello(
            1,
            0,
            PeerRole.DESKTOP,
            "x".repeat(ApplicationMessageCodec.MAX_DISPLAY_NAME_LENGTH + 1),
        )
        assertEncodeRejected(tooLong)

        assertEncodeRejected(
            ApplicationMessage.PairingHello(1, 256, PeerRole.DESKTOP, "Linux"),
        )
        assertEncodeRejected(
            ApplicationMessage.PairingHello(1, 0, PeerRole.DESKTOP, "WiPad\nDesktop"),
        )
    }

    private fun assertDecodeRejected(payload: ByteArray) {
        try {
            ApplicationMessageCodec.decode(payload)
            fail("Expected IOException")
        } catch (_: IOException) {
            // Expected.
        }
    }

    private fun assertEncodeRejected(message: ApplicationMessage) {
        try {
            ApplicationMessageCodec.encode(message)
            fail("Expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // Expected.
        }
    }
}
