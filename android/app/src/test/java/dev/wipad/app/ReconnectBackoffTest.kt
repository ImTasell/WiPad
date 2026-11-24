package dev.wipad.app

import org.junit.Assert.assertEquals
import org.junit.Test

class ReconnectBackoffTest {
    @Test
    fun growsExponentiallyAndCapsAtThirtySeconds() {
        assertEquals(1_000L, ReconnectBackoff.delayMillis(0))
        assertEquals(2_000L, ReconnectBackoff.delayMillis(1))
        assertEquals(4_000L, ReconnectBackoff.delayMillis(2))
        assertEquals(8_000L, ReconnectBackoff.delayMillis(3))
        assertEquals(16_000L, ReconnectBackoff.delayMillis(4))
        assertEquals(30_000L, ReconnectBackoff.delayMillis(5))
        assertEquals(30_000L, ReconnectBackoff.delayMillis(50))
    }

    @Test
    fun addsBoundedJitterWithoutExceedingCap() {
        assertEquals(1_250L, ReconnectBackoff.delayMillis(0, 250))
        assertEquals(1_500L, ReconnectBackoff.delayMillis(0, 5_000))
        assertEquals(30_000L, ReconnectBackoff.delayMillis(5, 500))
    }
}
