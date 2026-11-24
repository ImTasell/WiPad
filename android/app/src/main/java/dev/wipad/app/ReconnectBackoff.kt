package dev.wipad.app

internal object ReconnectBackoff {
    private const val INITIAL_DELAY_MILLIS = 1_000L
    private const val MAX_DELAY_MILLIS = 30_000L
    private const val MAX_EXPONENT = 5

    fun delayMillis(attempt: Int, jitterMillis: Long = 0L): Long {
        val exponent = attempt.coerceIn(0, MAX_EXPONENT)
        val base = (INITIAL_DELAY_MILLIS shl exponent).coerceAtMost(MAX_DELAY_MILLIS)
        return (base + jitterMillis.coerceIn(0L, 500L)).coerceAtMost(MAX_DELAY_MILLIS)
    }
}
