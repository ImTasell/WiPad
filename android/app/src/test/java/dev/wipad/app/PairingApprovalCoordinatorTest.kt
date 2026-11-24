package dev.wipad.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingApprovalCoordinatorTest {
    private fun prompt() = PairingPrompt(
        peerDisplayName = "WiPad Desktop",
        comparisonString = "A1B2-C3D4-E5F6",
        peerFingerprint = ByteArray(32) { 7 },
        reconnecting = false,
    )

    @Test
    fun waitsForAnExplicitApprovalDecision() {
        val coordinator = PairingApprovalCoordinator(timeoutMillis = 2_000)
        val result = java.util.concurrent.atomic.AtomicBoolean(false)
        val worker = Thread { result.set(coordinator.requestApproval(prompt())) }
        worker.start()

        val deadline = System.nanoTime() + 1_000_000_000L
        while (coordinator.pendingRequest() == null && System.nanoTime() < deadline) {
            Thread.yield()
        }

        assertNotNull(coordinator.pendingRequest())
        assertTrue(coordinator.decide(true))
        worker.join(1_000)
        assertFalse(worker.isAlive)
        assertTrue(result.get())
        assertNull(coordinator.pendingRequest())
    }

    @Test
    fun rejectionAndTimeoutFailClosed() {
        val coordinator = PairingApprovalCoordinator(timeoutMillis = 20)
        val result = java.util.concurrent.atomic.AtomicBoolean(true)
        val worker = Thread { result.set(coordinator.requestApproval(prompt())) }
        worker.start()
        val deadline = System.nanoTime() + 1_000_000_000L
        while (coordinator.pendingRequest() == null && System.nanoTime() < deadline) {
            Thread.yield()
        }
        assertTrue(coordinator.decide(false))
        worker.join(1_000)
        assertFalse(result.get())

        assertFalse(coordinator.requestApproval(prompt()))
        assertNull(coordinator.pendingRequest())
    }

    @Test
    fun refusesConcurrentApprovalPrompts() {
        val coordinator = PairingApprovalCoordinator(timeoutMillis = 2_000)
        val worker = Thread { coordinator.requestApproval(prompt()) }
        worker.start()
        val deadline = System.nanoTime() + 1_000_000_000L
        while (coordinator.pendingRequest() == null && System.nanoTime() < deadline) {
            Thread.yield()
        }

        assertFalse(coordinator.requestApproval(prompt()))
        assertTrue(coordinator.decide(false))
        worker.join(1_000)
        assertEquals(null, coordinator.pendingRequest())
    }
}
