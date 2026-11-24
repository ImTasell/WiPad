package dev.wipad.app

import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

/**
 * Bridges a backend pairing request to a future approval surface.
 *
 * The network worker waits for a deliberate decision. Missing UI, timeout,
 * cancellation, or a second simultaneous request all fail closed.
 */
internal class PairingApprovalCoordinator(
    private val timeoutMillis: Long = DEFAULT_TIMEOUT_MILLIS,
    private val onPendingChanged: (PairingPrompt?) -> Unit = {},
) {
    private data class Pending(
        val prompt: PairingPrompt,
        val decision: CompletableFuture<Boolean>,
    )

    private val lock = Any()
    private var pending: Pending? = null

    fun requestApproval(prompt: PairingPrompt): Boolean {
        if (prompt.reconnecting) return true
        val request = Pending(prompt, CompletableFuture())
        synchronized(lock) {
            if (pending != null) return false
            pending = request
        }
        onPendingChanged(prompt)

        return try {
            request.decision.get(timeoutMillis, TimeUnit.MILLISECONDS)
        } catch (_: TimeoutException) {
            false
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        } catch (_: Exception) {
            false
        } finally {
            synchronized(lock) {
                if (pending === request) pending = null
            }
            onPendingChanged(null)
        }
    }

    fun pendingRequest(): PairingPrompt? = synchronized(lock) { pending?.prompt }

    fun decide(approved: Boolean): Boolean {
        val request = synchronized(lock) { pending } ?: return false
        return request.decision.complete(approved)
    }

    fun cancelPending() {
        decide(false)
    }

    companion object {
        const val DEFAULT_TIMEOUT_MILLIS = 120_000L
    }
}
