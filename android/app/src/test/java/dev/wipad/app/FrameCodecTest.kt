package dev.wipad.app

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.fail
import org.junit.Test

class FrameCodecTest {
    private val payload = GoldenVectors.bytes("frame_sample").copyOfRange(4, 7)
    private val encoded = GoldenVectors.bytes("frame_sample")

    @Test
    fun writesAndReadsTheCanonicalBigEndianFrameVector() {
        val output = ByteArrayOutputStream()
        FrameCodec.writeFrame(output, payload)
        assertArrayEquals(encoded, output.toByteArray())
        assertArrayEquals(payload, FrameCodec.readFrame(ByteArrayInputStream(encoded)))
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
    fun rejectsSharedMalformedFrameVectors() {
        listOf(
            "invalid_frame_empty",
            "invalid_frame_oversized",
            "invalid_frame_truncated",
        ).forEach { vectorName ->
            assertReadRejected(GoldenVectors.bytes(vectorName))
        }
    }

    @Test
    fun rejectsOversizedFramesWhenWriting() {
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

    private fun assertReadRejected(bytes: ByteArray) {
        try {
            FrameCodec.readFrame(ByteArrayInputStream(bytes))
            fail("Expected IOException")
        } catch (_: IOException) {
            // Expected.
        }
    }
}
