package dev.wipad.app

import java.io.IOException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class ApplicationMessageCodecTest {
    private val helloVector = byteArrayOf(
        0x01, 0x01, 0x00, 0x01, 0x05,
        'P'.code.toByte(), 'h'.code.toByte(), 'o'.code.toByte(),
        'n'.code.toByte(), 'e'.code.toByte(),
    )

    @Test
    fun encodesTheSharedPhonePairingHelloVector() {
        val message = ApplicationMessage.PairingHello(
            major = 1,
            minor = 1,
            role = PeerRole.PHONE,
            displayName = "Phone",
        )

        assertArrayEquals(helloVector, ApplicationMessageCodec.encode(message))
    }

    @Test
    fun decodesTheSharedPhonePairingHelloVector() {
        assertEquals(
            ApplicationMessage.PairingHello(1, 1, PeerRole.PHONE, "Phone"),
            ApplicationMessageCodec.decode(helloVector),
        )
    }

    @Test
    fun encodesAndDecodesPairingDecisionsAndReady() {
        for (approved in listOf(false, true)) {
            val message = ApplicationMessage.PairingDecision(approved)
            val vector = byteArrayOf(0x02, if (approved) 1 else 0)

            assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
            assertEquals(message, ApplicationMessageCodec.decode(vector))
        }

        assertArrayEquals(
            byteArrayOf(0x03),
            ApplicationMessageCodec.encode(ApplicationMessage.SessionReady),
        )
        assertEquals(
            ApplicationMessage.SessionReady,
            ApplicationMessageCodec.decode(byteArrayOf(0x03)),
        )
    }

    @Test
    fun encodesAndDecodesControllerPresenceStatus() {
        for ((connected, expected) in listOf(
            false to byteArrayOf(0x11, 0),
            true to byteArrayOf(0x11, 1),
        )) {
            val message = ApplicationMessage.ControllerStatus(connected)
            assertArrayEquals(expected, ApplicationMessageCodec.encode(message))
            assertEquals(message, ApplicationMessageCodec.decode(expected))
        }
    }

    @Test
    fun rejectsMalformedControllerPresenceStatus() {
        listOf(
            byteArrayOf(0x11),
            byteArrayOf(0x11, 2),
            byteArrayOf(0x11, 1, 0),
        ).forEach(::assertDecodeRejected)
    }

    @Test
    fun rejectsMalformedPairingMessages() {
        val invalidMessages = listOf(
            byteArrayOf(0x01),
            byteArrayOf(0x01, 2, 0, 1, 1, 'P'.code.toByte()),
            byteArrayOf(0x01, 1, 0, 9, 1, 'P'.code.toByte()),
            byteArrayOf(0x01, 1, 0, 1, 0),
            byteArrayOf(0x02, 2),
            byteArrayOf(0x02, 1, 0),
            byteArrayOf(0x03, 0),
            byteArrayOf(0x7F),
        )

        invalidMessages.forEach(::assertDecodeRejected)
    }

    @Test
    fun rejectsInvalidDisplayNames() {
        val tooLong = ApplicationMessage.PairingHello(
            1,
            0,
            PeerRole.DESKTOP,
            "x".repeat(ApplicationMessageCodec.MAX_DISPLAY_NAME_LENGTH + 1),
        )
        try {
            ApplicationMessageCodec.encode(tooLong)
            fail("Expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // Expected.
        }

        assertDecodeRejected(
            byteArrayOf(0x01, 1, 0, 2, 2, 'W'.code.toByte(), 0x0A),
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
}
