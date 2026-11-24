package dev.wipad.app

import java.io.File
import java.io.IOException

/**
 * Reads the repository-wide protocol fixtures shared with the Rust test suite.
 *
 * Gradle's Android unit-test worker normally runs from android/app; the
 * additional candidates also support invocation from the repository root.
 */
internal object GoldenVectors {
    private val entries: Map<String, String> by lazy {
        val fixture = listOf(
            File("protocol/golden-vectors.txt"),
            File("../../protocol/golden-vectors.txt"),
            File("../protocol/golden-vectors.txt"),
        ).firstOrNull(File::isFile)
            ?: throw IllegalStateException("Could not locate protocol/golden-vectors.txt")

        fixture.readText(Charsets.UTF_8).lineSequence()
            .map(String::trim)
            .filter { it.isNotEmpty() && !it.startsWith("#") }
            .associate { line ->
                val separator = line.indexOf('=')
                require(separator > 0) { "Malformed golden vector: $line" }
                line.substring(0, separator) to line.substring(separator + 1)
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
