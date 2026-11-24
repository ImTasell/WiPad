package dev.wipad.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TlsFingerprintTest {
    @Test
    fun certificateFingerprintIsSha256Sized() {
        val fingerprint = TlsFingerprint.certificateFingerprint(byteArrayOf(1, 2, 3))

        assertEquals(32, fingerprint.size)
    }

    @Test
    fun pairingStringUsesSharedPhoneThenDesktopOrdering() {
        val phoneCertificate = ByteArray(32) { it.toByte() }
        val desktopCertificate = ByteArray(32) { (31 - it).toByte() }

        val pairing = TlsFingerprint.pairingString(phoneCertificate, desktopCertificate)

        assertEquals("8223-AE56-2CDD", pairing)
        assertEquals(
            pairing,
            TlsFingerprint.pairingString(phoneCertificate, desktopCertificate),
        )
        assertFalse(
            pairing == TlsFingerprint.pairingString(desktopCertificate, phoneCertificate),
        )
    }

    @Test
    fun certificatePinsUseConstantTimeEqualityAndRejectInvalidLengths() {
        val pin = ByteArray(32) { it.toByte() }

        assertTrue(TlsFingerprint.matchesFingerprint(pin, pin.copyOf()))
        assertFalse(TlsFingerprint.matchesFingerprint(pin, ByteArray(32)))
        assertFalse(TlsFingerprint.matchesFingerprint(null, pin))
        assertFalse(TlsFingerprint.matchesFingerprint(ByteArray(31), pin))
    }

    @Test
    fun pairingStringRejectsEmptyCertificates() {
        try {
            TlsFingerprint.pairingString(byteArrayOf(), byteArrayOf(1))
            throw AssertionError("Expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // Expected.
        }
    }
}
