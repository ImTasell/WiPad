package dev.wipad.app

import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder

internal data class ControllerState(
    val sequence: Long,
    val sampleTimeMicros: Long,
    val buttons: Int,
    val hat: Int,
    val leftX: Int,
    val leftY: Int,
    val rightX: Int,
    val rightY: Int,
    val leftTrigger: Int,
    val rightTrigger: Int,
) {
    fun validate() {
        if (sequence <= 0L) throw IOException("Controller sequence must start at 1")
        if (buttons and BUTTON_MASK.inv() != 0) {
            throw IOException("Controller snapshot contains unknown button bits")
        }
        if (hat !in 0..8) throw IOException("Controller hat value must be in 0..=8")
        if (leftX !in AXIS_MIN..AXIS_MAX
            || leftY !in AXIS_MIN..AXIS_MAX
            || rightX !in AXIS_MIN..AXIS_MAX
            || rightY !in AXIS_MIN..AXIS_MAX
        ) {
            throw IOException("Controller stick axis is outside the signed 16-bit range")
        }
        if (leftTrigger !in 0..TRIGGER_MAX || rightTrigger !in 0..TRIGGER_MAX) {
            throw IOException("Controller trigger is outside the unsigned 16-bit range")
        }
    }

    companion object {
        const val BUTTON_SOUTH = 1 shl 0
        const val BUTTON_EAST = 1 shl 1
        const val BUTTON_WEST = 1 shl 2
        const val BUTTON_NORTH = 1 shl 3
        const val BUTTON_LEFT_BUMPER = 1 shl 4
        const val BUTTON_RIGHT_BUMPER = 1 shl 5
        const val BUTTON_BACK = 1 shl 6
        const val BUTTON_START = 1 shl 7
        const val BUTTON_GUIDE = 1 shl 8
        const val BUTTON_LEFT_STICK = 1 shl 9
        const val BUTTON_RIGHT_STICK = 1 shl 10

        const val BUTTON_MASK = (1 shl 11) - 1
        const val AXIS_MIN = Short.MIN_VALUE.toInt()
        const val AXIS_MAX = Short.MAX_VALUE.toInt()
        const val TRIGGER_MAX = 0xFFFF

        fun neutral(sequence: Long, sampleTimeMicros: Long) = ControllerState(
            sequence = sequence,
            sampleTimeMicros = sampleTimeMicros,
            buttons = 0,
            hat = 0,
            leftX = 0,
            leftY = 0,
            rightX = 0,
            rightY = 0,
            leftTrigger = 0,
            rightTrigger = 0,
        )
    }
}

internal object ControllerStateCodec {
    const val MESSAGE_TYPE = 0x10
    const val PAYLOAD_LENGTH = 34

    fun encode(state: ControllerState): ByteArray {
        state.validate()

        return ByteBuffer.allocate(PAYLOAD_LENGTH)
            .order(ByteOrder.BIG_ENDIAN)
            .put(MESSAGE_TYPE.toByte())
            .putLong(state.sequence)
            .putLong(state.sampleTimeMicros)
            .putInt(state.buttons)
            .put(state.hat.toByte())
            .putShort(state.leftX.toShort())
            .putShort(state.leftY.toShort())
            .putShort(state.rightX.toShort())
            .putShort(state.rightY.toShort())
            .putShort(state.leftTrigger.toShort())
            .putShort(state.rightTrigger.toShort())
            .array()
    }

    @Throws(IOException::class)
    fun decode(payload: ByteArray): ControllerState {
        if (payload.size != PAYLOAD_LENGTH
            || (payload[0].toInt() and 0xFF) != MESSAGE_TYPE
        ) {
            throw IOException("Controller snapshot length or message type is invalid")
        }

        val buffer = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN)
        buffer.get() // Message type.
        val state = ControllerState(
            sequence = buffer.long,
            sampleTimeMicros = buffer.long,
            buttons = buffer.int,
            hat = buffer.get().toInt() and 0xFF,
            leftX = buffer.short.toInt(),
            leftY = buffer.short.toInt(),
            rightX = buffer.short.toInt(),
            rightY = buffer.short.toInt(),
            leftTrigger = buffer.short.toInt() and 0xFFFF,
            rightTrigger = buffer.short.toInt() and 0xFFFF,
        )
        state.validate()
        return state
    }
}

/** Rejects duplicate and older controller snapshots within one authenticated session. */
internal class InputSequenceTracker {
    private var lastSequence: Long? = null

    fun accept(state: ControllerState): Boolean {
        state.validate()
        val previous = lastSequence
        if (previous != null && state.sequence <= previous) return false
        lastSequence = state.sequence
        return true
    }

    fun reset() {
        lastSequence = null
    }

    fun lastSequence(): Long? = lastSequence
}
