package dev.wipad.app

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Test

class ControllerInputMathTest {
    @Test
    fun mapsAndroidGamepadButtonsToTheSharedWireBits() {
        assertEquals(ControllerState.BUTTON_SOUTH, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_A))
        assertEquals(ControllerState.BUTTON_EAST, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_B))
        assertEquals(ControllerState.BUTTON_WEST, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_X))
        assertEquals(ControllerState.BUTTON_NORTH, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_Y))
        assertEquals(ControllerState.BUTTON_LEFT_BUMPER, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_L1))
        assertEquals(ControllerState.BUTTON_RIGHT_BUMPER, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_R1))
        assertEquals(ControllerState.BUTTON_BACK, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_SELECT))
        assertEquals(ControllerState.BUTTON_START, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_START))
        assertEquals(ControllerState.BUTTON_GUIDE, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_MODE))
        assertEquals(ControllerState.BUTTON_LEFT_STICK, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_THUMBL))
        assertEquals(ControllerState.BUTTON_RIGHT_STICK, ControllerInputMath.buttonBit(KeyEvent.KEYCODE_BUTTON_THUMBR))
    }

    @Test
    fun mapsAllHatDirections() {
        val expected = listOf(
            0 to 0,
            0 to -1,
            1 to -1,
            1 to 0,
            1 to 1,
            0 to 1,
            -1 to 1,
            -1 to 0,
            -1 to -1,
        )
        expected.forEachIndexed { hat, axes ->
            assertEquals(hat, ControllerInputMath.hatValue(axes.first, axes.second))
        }
    }

    @Test
    fun normalizesStickAndTriggerRanges() {
        assertEquals(0, ControllerInputMath.normalizeStick(0f, -1f, 1f, 0f))
        assertEquals(Short.MIN_VALUE.toInt(), ControllerInputMath.normalizeStick(-1f, -1f, 1f, 0f))
        assertEquals(Short.MAX_VALUE.toInt(), ControllerInputMath.normalizeStick(1f, -1f, 1f, 0f))
        assertEquals(0, ControllerInputMath.normalizeTrigger(-1f, -1f, 1f))
        assertEquals(65535, ControllerInputMath.normalizeTrigger(1f, -1f, 1f))
    }
}
