package dev.wipad.app

import java.io.IOException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class ControllerStateCodecTest {
    private val vector = GoldenVectors.bytes("input_state_sample")

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
    fun encodesAndDecodesTheCanonicalControllerStateVector() {
        assertArrayEquals(vector, ControllerStateCodec.encode(state))
        assertEquals(state, ControllerStateCodec.decode(vector))
    }

    @Test
    fun applicationMessageCodecCarriesTheCanonicalInputState() {
        val message = ApplicationMessage.InputState(state)
        assertArrayEquals(vector, ApplicationMessageCodec.encode(message))
        assertEquals(message, ApplicationMessageCodec.decode(vector))
    }

    @Test
    fun rejectsMalformedControllerStateAndSharedInvalidVector() {
        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(vector.copyOf(33))
        }

        val invalidType = vector.copyOf()
        invalidType[0] = 0x11
        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(invalidType)
        }

        assertThrows(IOException::class.java) {
            ControllerStateCodec.decode(GoldenVectors.bytes("invalid_input_state_hat"))
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
