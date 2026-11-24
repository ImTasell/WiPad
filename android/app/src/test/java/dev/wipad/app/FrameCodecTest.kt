package dev.wipad.app

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.fail
import org.junit.Test

class FrameCodecTest {
    // Shared wire vector: payload 10 20 30 -> 00 00 00 03 10 20 30.
    private val payload = byteArrayOf(0x10, 0x20, 0x30)
    private val encoded = byteArrayOf(0x00, 0x00, 0x00, 0x03, 0x10, 0x20, 0x30)

    @Test
    fun writesTheSharedBigEndianVector() {
        val output = ByteArrayOutputStream()

        FrameCodec.writeFrame(output, payload)

        assertArrayEquals(encoded, output.toByteArray())
    }

    @Test
    fun readsTheSharedBigEndianVector() {
        assertArrayEquals(
            payload,
            FrameCodec.readFrame(ByteArrayInputStream(encoded)),
        )
    }

    @Test
    fun readsFragmentedStreams() {
        val fragmented = object : InputStream() {
            private val source = ByteArrayInputStream(encoded)

            override fun read(): Int = source.read()

            override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
                source.read(buffer, offset, minOf(length, 1))
        }

        assertArrayEquals(payload, FrameCodec.readFrame(fragmented))
    }

    @Test
    fun rejectsEmptyFrames() {
        assertReadRejected(byteArrayOf(0, 0, 0, 0))

        try {
            FrameCodec.writeFrame(ByteArrayOutputStream(), byteArrayOf())
            fail("Expected IOException")
        } catch (_: IOException) {
            // Expected.
        }
    }

    @Test
    fun rejectsOversizedFramesBeforeReadingThePayload() {
        val length = FrameCodec.MAX_FRAME_LENGTH + 1
        val header = byteArrayOf(
            0,
            0,
            (length ushr 8).toByte(),
            length.toByte(),
        )
        assertReadRejected(header)

        try {
            FrameCodec.writeFrame(
                ByteArrayOutputStream(),
                ByteArray(FrameCodec.MAX_FRAME_LENGTH + 1),
            )
            fail("Expected IOException")
        } catch (_: IOException) {
            // Expected.
        }
    }

    @Test
    fun rejectsTruncatedPayloads() {
        assertReadRejected(byteArrayOf(0, 0, 0, 3, 0x10, 0x20))
    }

    private fun assertReadRejected(bytes: ByteArray) {
        try {
            FrameCodec.readFrame(ByteArrayInputStream(bytes))
            fail("Expected IOException")
        } catch (_: IOException) {
            // Expected.
        }
    }
}
