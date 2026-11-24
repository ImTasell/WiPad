package dev.wipad.app

import java.io.IOException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ControllerStateCodecTest {
    // Same 34-byte vector asserted by desktop/src/controller.rs.
    private val vector = byteArrayOf(
        0x10,
        0, 0, 0, 0, 0, 0, 0, 1,
        0, 0, 0, 0, 0, 0, 0, 2,
        0, 0, 0, 9,
        0,
        0x12, 0x34, 0xFF.toByte(), 0xFE.toByte(),
        0, 0, 0x80.toByte(), 0,
        0xFF.toByte(), 0xFF.toByte(), 0, 0,
    )

    private val state = ControllerState(
        sequence = 1,
        sampleTimeMicros = 2,
        buttons = ControllerState.BUTTON_SOUTH or ControllerState.BUTTON_NORTH,
        hat = 0,
        leftX = 0x1234,
        leftY = -2,
        rightX = 0,
        rightY = Short.MIN_VALUE.toInt(),
        leftTrigger = ControllerState.TRIGGER_MAX,
        rightTrigger = 0,
    )

    @Test
    fun encodesTheSharedControllerStateVector() {
        assertArrayEquals(vector, ControllerStateCodec.encode(state))
    }

    @Test
    fun decodesTheSharedControllerStateVector() {
        assertEquals(state, ControllerStateCodec.decode(vector))
    }

    @Test
    fun applicationMessageCodecCarriesInputState() {
        val message = ApplicationMessage.InputState(state)
        assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
        assertEquals(message, ApplicationMessageCodec.decode(vector))
    }

    @Test
    fun rejectsMalformedControllerState() {
        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(vector.copyOf(33))
        }

        val invalidType = vector.copyOf()
        invalidType[0] = 0x11
        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(invalidType)
        }

        val invalidHat = vector.copyOf()
        invalidHat[21] = 9
        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(invalidHat)
        }

        assertThrows(IOException::class.java) {
            ControllerStateCodec.encode(state.copy(sequence = 0))
        }
        assertThrows(IOException::class.java) {
            ControllerStateCodec.encode(state.copy(buttons = 1 shl 15))
        }
    }

    @Test
    fun sequenceTrackerRejectsDuplicateAndOlderSnapshots() {
        val tracker = InputSequenceTracker()

        assertTrue(tracker.accept(state))
        assertFalse(tracker.accept(state))
        assertTrue(tracker.accept(state.copy(sequence = 3)))
        assertFalse(tracker.accept(state.copy(sequence = 2)))
        assertEquals(3L, tracker.lastSequence())

        tracker.reset()
        assertTrue(tracker.accept(state))
    }
}
