package dev.wipad.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingGateTest {
    @Test
    fun requiresBothSidesToApproveBeforeReady() {
        val gate = PairingGate(localPinMatches = false)

        assertEquals(PairingState.AWAITING_BOTH_APPROVALS, gate.state())
        assertFalse(gate.canStorePeerPin())
        assertFalse(gate.canStartInput())

        gate.approveLocal()
        assertEquals(PairingState.AWAITING_PEER_APPROVAL, gate.state())
        assertFalse(gate.canStartInput())

        gate.receivePeerDecision(approved = true)
        assertEquals(PairingState.READY, gate.state())
        assertTrue(gate.canStorePeerPin())
        assertTrue(gate.canStartInput())
    }

    @Test
    fun matchingStoredPinApprovesOnlyLocalSide() {
        val gate = PairingGate(localPinMatches = true)

        assertEquals(PairingState.AWAITING_PEER_APPROVAL, gate.state())
        assertFalse(gate.canStartInput())

        gate.receivePeerDecision(approved = true)
        assertEquals(PairingState.READY, gate.state())
        assertTrue(gate.canStartInput())
    }

    @Test
    fun remoteApprovalWaitsForLocalApproval() {
        val gate = PairingGate(localPinMatches = false)
        gate.receivePeerDecision(approved = true)

        assertEquals(PairingState.AWAITING_LOCAL_APPROVAL, gate.state())
        assertFalse(gate.canStartInput())

        gate.approveLocal()
        assertEquals(PairingState.READY, gate.state())
    }

    @Test
    fun rejectionPermanentlyBlocksTheSession() {
        val gate = PairingGate(localPinMatches = true)
        gate.receivePeerDecision(approved = false)
        gate.approveLocal()
        gate.receivePeerDecision(approved = true)

        assertEquals(PairingState.REJECTED, gate.state())
        assertFalse(gate.canStorePeerPin())
        assertFalse(gate.canStartInput())
    }
}
