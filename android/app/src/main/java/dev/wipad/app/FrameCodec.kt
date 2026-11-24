package dev.wipad.app

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

/**
 * Length-prefixed framing for the WiPad TCP byte stream.
 *
 * The four-byte unsigned payload length is big-endian and excludes the prefix.
 */
internal object FrameCodec {
    const val MAX_FRAME_LENGTH = 4 * 1024

    @Throws(IOException::class)
    fun readFrame(input: InputStream): ByteArray {
        val data = DataInputStream(input)
        val length = data.readInt().toLong() and 0xFFFF_FFFFL
        validateLength(length)

        return ByteArray(length.toInt()).also(data::readFully)
    }

    @Throws(IOException::class)
    fun writeFrame(output: OutputStream, payload: ByteArray) {
        validateLength(payload.size.toLong())

        val data = DataOutputStream(output)
        data.writeInt(payload.size)
        data.write(payload)
        data.flush()
    }

    private fun validateLength(length: Long) {
        if (length !in 1L..MAX_FRAME_LENGTH.toLong()) {
            throw IOException(
                "WiPad frame length must be between 1 and $MAX_FRAME_LENGTH bytes",
            )
        }
    }
}
