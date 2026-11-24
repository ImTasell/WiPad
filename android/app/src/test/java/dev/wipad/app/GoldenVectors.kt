package dev.wipad.app

import java.io.IOException

/**
 * Reads the repository-wide protocol fixtures shared with the Rust test suite.
 */
internal object GoldenVectors {
    private val entries: Map<String, String> by lazy {
        val stream = checkNotNull(
            GoldenVectors::class.java.getResourceAsStream("/golden-vectors.txt"),
        ) { "golden-vectors.txt was not added to the Android test resources" }

        stream.bufferedReader(Charsets.UTF_8).useLines { lines ->
            lines.map(String::trim)
                .filter { it.isNotEmpty() && !it.startsWith("#") }
                .associate { line ->
                    val separator = line.indexOf('=')
                    require(separator > 0) { "Malformed golden vector: $line" }
                    line.substring(0, separator) to line.substring(separator + 1)
                }
        }
    }

    fun bytes(name: String): ByteArray {
        val hex = entries[name] ?: throw IOException("Missing WiPad golden vector: $name")
        if (hex.length % 2 != 0 || hex.any { it.digitToIntOrNull(16) == null }) {
            throw IOException("Malformed WiPad golden vector: $name")
        }
        return ByteArray(hex.length / 2) { index ->
            hex.substring(index * 2, index * 2 + 2).toInt(16).toByte()
        }
    }
}
