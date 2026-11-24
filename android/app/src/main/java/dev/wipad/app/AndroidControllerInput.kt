package dev.wipad.app

import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Collects one physical Android gamepad and normalizes it into the platform-neutral
 * WiPad layout. Device IDs and Android key codes never leave the phone.
 */
internal class AndroidControllerInput {
    private var selectedDeviceId: Int? = null
    private var buttons = 0
    private var leftX = 0
    private var leftY = 0
    private var rightX = 0
    private var rightY = 0
    private var leftTrigger = 0
    private var rightTrigger = 0
    private var motionHatX = 0
    private var motionHatY = 0
    private var dpadUp = false
    private var dpadDown = false
    private var dpadLeft = false
    private var dpadRight = false

    @Synchronized
    fun refreshDevices(): Int? {
        val current = selectedDeviceId?.let(InputDevice::getDevice)
        if (current != null && isPhysicalGamepad(current)) return selectedDeviceId

        val candidate = InputDevice.getDeviceIds()
            .asSequence()
            .mapNotNull(InputDevice::getDevice)
            .firstOrNull(::isPhysicalGamepad)

        if (candidate?.id != selectedDeviceId) {
            selectedDeviceId = candidate?.id
            resetState()
        }
        return selectedDeviceId
    }

    @Synchronized
    fun onDeviceRemoved(deviceId: Int) {
        if (selectedDeviceId == deviceId) {
            selectedDeviceId = null
            resetState()
            refreshDevices()
        }
    }

    @Synchronized
    fun handleKeyEvent(event: KeyEvent): Boolean {
        val selected = selectedDeviceId ?: return false
        if (event.deviceId != selected) return false

        val bit = ControllerInputMath.buttonBit(event.keyCode)
        if (bit != 0) {
            when (event.action) {
                KeyEvent.ACTION_DOWN -> buttons = buttons or bit
                KeyEvent.ACTION_UP -> buttons = buttons and bit.inv()
                else -> return false
            }
            return true
        }

        val pressed = when (event.action) {
            KeyEvent.ACTION_DOWN -> true
            KeyEvent.ACTION_UP -> false
            else -> return false
        }

        when (event.keyCode) {
            KeyEvent.KEYCODE_DPAD_UP -> dpadUp = pressed
            KeyEvent.KEYCODE_DPAD_DOWN -> dpadDown = pressed
            KeyEvent.KEYCODE_DPAD_LEFT -> dpadLeft = pressed
            KeyEvent.KEYCODE_DPAD_RIGHT -> dpadRight = pressed
            else -> return false
        }
        return true
    }

    @Synchronized
    fun handleMotionEvent(event: MotionEvent): Boolean {
        val selected = selectedDeviceId ?: return false
        if (event.deviceId != selected) return false
        if (event.source and InputDevice.SOURCE_JOYSTICK != InputDevice.SOURCE_JOYSTICK) {
            return false
        }

        val device = InputDevice.getDevice(selected) ?: return false
        val source = event.source

        leftX = signedAxis(event, device, MotionEvent.AXIS_X, source)
        leftY = signedAxis(event, device, MotionEvent.AXIS_Y, source)
        val rightXAxis = if (device.getMotionRange(MotionEvent.AXIS_Z, source) != null) {
            MotionEvent.AXIS_Z
        } else {
            MotionEvent.AXIS_RX
        }
        val rightYAxis = if (device.getMotionRange(MotionEvent.AXIS_RZ, source) != null) {
            MotionEvent.AXIS_RZ
        } else {
            MotionEvent.AXIS_RY
        }
        rightX = signedAxis(event, device, rightXAxis, source)
        rightY = signedAxis(event, device, rightYAxis, source)

        val leftTriggerAxis = when {
            device.getMotionRange(MotionEvent.AXIS_LTRIGGER, source) != null -> MotionEvent.AXIS_LTRIGGER
            device.getMotionRange(MotionEvent.AXIS_BRAKE, source) != null -> MotionEvent.AXIS_BRAKE
            else -> null
        }
        val rightTriggerAxis = when {
            device.getMotionRange(MotionEvent.AXIS_RTRIGGER, source) != null -> MotionEvent.AXIS_RTRIGGER
            device.getMotionRange(MotionEvent.AXIS_GAS, source) != null -> MotionEvent.AXIS_GAS
            else -> null
        }
        leftTrigger = leftTriggerAxis?.let { triggerAxis(event, device, it, source) } ?: 0
        rightTrigger = rightTriggerAxis?.let { triggerAxis(event, device, it, source) } ?: 0

        motionHatX = event.getAxisValue(MotionEvent.AXIS_HAT_X).roundToInt().coerceIn(-1, 1)
        motionHatY = event.getAxisValue(MotionEvent.AXIS_HAT_Y).roundToInt().coerceIn(-1, 1)
        return true
    }

    @Synchronized
    fun snapshot(sequence: Long, sampleTimeMicros: Long): ControllerState {
        val keyHatX = when {
            dpadLeft && !dpadRight -> -1
            dpadRight && !dpadLeft -> 1
            else -> 0
        }
        val keyHatY = when {
            dpadUp && !dpadDown -> -1
            dpadDown && !dpadUp -> 1
            else -> 0
        }
        val hatX = if (motionHatX != 0) motionHatX else keyHatX
        val hatY = if (motionHatY != 0) motionHatY else keyHatY

        return ControllerState(
            sequence = sequence,
            sampleTimeMicros = sampleTimeMicros,
            buttons = buttons,
            hat = ControllerInputMath.hatValue(hatX, hatY),
            leftX = leftX,
            leftY = leftY,
            rightX = rightX,
            rightY = rightY,
            leftTrigger = leftTrigger,
            rightTrigger = rightTrigger,
        )
    }

    private fun signedAxis(
        event: MotionEvent,
        device: InputDevice,
        axis: Int,
        source: Int,
    ): Int {
        val range = device.getMotionRange(axis, source)
            ?: device.getMotionRange(axis)
            ?: return 0
        return ControllerInputMath.normalizeStick(
            event.getAxisValue(axis),
            range.min,
            range.max,
            range.flat,
        )
    }

    private fun triggerAxis(
        event: MotionEvent,
        device: InputDevice,
        axis: Int,
        source: Int,
    ): Int {
        val range = device.getMotionRange(axis, source)
            ?: device.getMotionRange(axis)
            ?: return 0
        return ControllerInputMath.normalizeTrigger(
            event.getAxisValue(axis),
            range.min,
            range.max,
        )
    }

    private fun isPhysicalGamepad(device: InputDevice): Boolean =
        !device.isVirtual && (
            device.supportsSource(InputDevice.SOURCE_GAMEPAD)
                || device.supportsSource(InputDevice.SOURCE_JOYSTICK)
            )

    private fun resetState() {
        buttons = 0
        leftX = 0
        leftY = 0
        rightX = 0
        rightY = 0
        leftTrigger = 0
        rightTrigger = 0
        motionHatX = 0
        motionHatY = 0
        dpadUp = false
        dpadDown = false
        dpadLeft = false
        dpadRight = false
    }
}

internal object ControllerInputMath {
    fun buttonBit(keyCode: Int): Int = when (keyCode) {
        KeyEvent.KEYCODE_BUTTON_A -> ControllerState.BUTTON_SOUTH
        KeyEvent.KEYCODE_BUTTON_B -> ControllerState.BUTTON_EAST
        KeyEvent.KEYCODE_BUTTON_X -> ControllerState.BUTTON_WEST
        KeyEvent.KEYCODE_BUTTON_Y -> ControllerState.BUTTON_NORTH
        KeyEvent.KEYCODE_BUTTON_L1 -> ControllerState.BUTTON_LEFT_BUMPER
        KeyEvent.KEYCODE_BUTTON_R1 -> ControllerState.BUTTON_RIGHT_BUMPER
        KeyEvent.KEYCODE_BUTTON_SELECT -> ControllerState.BUTTON_BACK
        KeyEvent.KEYCODE_BUTTON_START -> ControllerState.BUTTON_START
        KeyEvent.KEYCODE_BUTTON_MODE -> ControllerState.BUTTON_GUIDE
        KeyEvent.KEYCODE_BUTTON_THUMBL -> ControllerState.BUTTON_LEFT_STICK
        KeyEvent.KEYCODE_BUTTON_THUMBR -> ControllerState.BUTTON_RIGHT_STICK
        else -> 0
    }

    fun hatValue(x: Int, y: Int): Int = when (x.coerceIn(-1, 1) to y.coerceIn(-1, 1)) {
        0 to -1 -> 1
        1 to -1 -> 2
        1 to 0 -> 3
        1 to 1 -> 4
        0 to 1 -> 5
        -1 to 1 -> 6
        -1 to 0 -> 7
        -1 to -1 -> 8
        else -> 0
    }

    fun normalizeStick(value: Float, minimum: Float, maximum: Float, flat: Float): Int {
        if (!value.isFinite() || maximum <= minimum) return 0
        val center = (minimum + maximum) / 2f
        val normalized = if (value < center) {
            (value - center) / (center - minimum)
        } else {
            (value - center) / (maximum - center)
        }.coerceIn(-1f, 1f)

        val deadZone = (flat / ((maximum - minimum) / 2f)).coerceIn(0f, 0.95f)
        val adjusted = if (abs(normalized) <= deadZone) {
            0f
        } else {
            normalized.sign() * ((abs(normalized) - deadZone) / (1f - deadZone))
        }

        return if (adjusted < 0f) {
            (adjusted * 32768f).roundToInt().coerceAtLeast(Short.MIN_VALUE.toInt())
        } else {
            (adjusted * 32767f).roundToInt().coerceAtMost(Short.MAX_VALUE.toInt())
        }
    }

    fun normalizeTrigger(value: Float, minimum: Float, maximum: Float): Int {
        if (!value.isFinite() || maximum <= minimum) return 0
        val normalized = ((value - minimum) / (maximum - minimum)).coerceIn(0f, 1f)
        return (normalized * 65535f).roundToInt()
    }

    private fun Float.sign(): Float = when {
        this < 0f -> -1f
        this > 0f -> 1f
        else -> 0f
    }
}
