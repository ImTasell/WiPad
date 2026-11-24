package dev.wipad.app

/**
 * Requires local and remote approval before a WiPad session can become READY.
 *
 * A matching stored desktop pin counts as local approval. The phone still
 * waits for an authenticated approval decision from the desktop. A new
 * desktop requires an explicit local approval after the comparison string
 * has been checked on both screens.
 */
internal class PairingGate(localPinMatches: Boolean) {
    private var localApproved = localPinMatches
    private var peerApproved = false
    private var rejected = false

    @Synchronized
    fun approveLocal() {
        if (!rejected) localApproved = true
    }

    @Synchronized
    fun receivePeerDecision(approved: Boolean) {
        if (rejected) return

        if (approved) {
            peerApproved = true
        } else {
            rejected = true
        }
    }

    @Synchronized
    fun state(): PairingState = when {
        rejected -> PairingState.REJECTED
        localApproved && peerApproved -> PairingState.READY
        !localApproved && peerApproved -> PairingState.AWAITING_LOCAL_APPROVAL
        localApproved && !peerApproved -> PairingState.AWAITING_PEER_APPROVAL
        else -> PairingState.AWAITING_BOTH_APPROVALS
    }

    @Synchronized
    fun canStorePeerPin(): Boolean = state() == PairingState.READY

    @Synchronized
    fun canStartInput(): Boolean = canStorePeerPin()
}

internal enum class PairingState {
    AWAITING_BOTH_APPROVALS,
    AWAITING_LOCAL_APPROVAL,
    AWAITING_PEER_APPROVAL,
    READY,
    REJECTED,
}
