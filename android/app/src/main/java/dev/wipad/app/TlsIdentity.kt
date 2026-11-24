package dev.wipad.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.math.BigInteger
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.security.interfaces.ECPublicKey
import java.util.Date
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager
import javax.security.auth.x500.X500Principal

/**
 * Android Keystore-backed client identity for the WiPad TLS 1.3 session.
 *
 * The EC P-256 private key never leaves Android Keystore. Only the public
 * certificate is exposed so it can be fingerprinted and compared at pairing.
 */
class TlsIdentity private constructor(
    private val keyStore: KeyStore,
) {
    companion object {
        private const val KEYSTORE_PROVIDER = "AndroidKeyStore"
        private const val KEY_ALIAS = "dev.wipad.tls.p256"
        private const val KEY_ALGORITHM = KeyProperties.KEY_ALGORITHM_EC
        private const val CURVE_NAME = "secp256r1"

        fun loadOrCreate(): TlsIdentity {
            val keyStore = KeyStore.getInstance(KEYSTORE_PROVIDER).apply {
                load(null)
            }

            if (!keyStore.containsAlias(KEY_ALIAS)) {
                val now = System.currentTimeMillis()
                val generator = KeyPairGenerator.getInstance(KEY_ALGORITHM, KEYSTORE_PROVIDER)
                generator.initialize(
                    KeyGenParameterSpec.Builder(
                        KEY_ALIAS,
                        KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
                    )
                        .setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec(CURVE_NAME))
                        .setDigests(KeyProperties.DIGEST_SHA256)
                        .setCertificateSubject(X500Principal("CN=WiPad Phone"))
                        .setCertificateSerialNumber(
                            BigInteger(1, ByteArray(16).also(SecureRandom()::nextBytes)),
                        )
                        .setCertificateNotBefore(Date(now - DAY_MILLIS))
                        .setCertificateNotAfter(Date(now + CERTIFICATE_LIFETIME_MILLIS))
                        .setUserAuthenticationRequired(false)
                        .build(),
                )
                generator.generateKeyPair()
            }

            val certificate = keyStore.getCertificate(KEY_ALIAS) as? X509Certificate
                ?: throw CertificateException("Android Keystore did not return the WiPad certificate")
            validateLocalCertificate(certificate)

            return TlsIdentity(keyStore)
        }

        private const val DAY_MILLIS = 24L * 60L * 60L * 1000L
        private const val CERTIFICATE_LIFETIME_MILLIS = 3650L * DAY_MILLIS

        private fun validateLocalCertificate(certificate: X509Certificate) {
            certificate.checkValidity()

            val publicKey = certificate.publicKey as? ECPublicKey
                ?: throw CertificateException("WiPad identity must use an EC public key")
            if (publicKey.params.curve.field.fieldSize != 256) {
                throw CertificateException("WiPad identity must use an ECDSA P-256 key")
            }
        }
    }

    val certificate: X509Certificate
        get() = keyStore.getCertificate(KEY_ALIAS) as X509Certificate

    fun fingerprint(): ByteArray = TlsFingerprint.certificateFingerprint(certificate.encoded)

    /**
     * Creates a TLS 1.3 context which presents the Android Keystore certificate
     * and checks the desktop's certificate against the optional saved pin.
     *
     * A null pin is only valid during first-contact pairing. TLS still verifies
     * the desktop's TLS 1.3 CertificateVerify signature with its P-256 key.
     */
    fun createClientContext(expectedDesktopFingerprint: ByteArray?): SSLContext {
        val keyManagerFactory = KeyManagerFactory.getInstance(
            KeyManagerFactory.getDefaultAlgorithm(),
        )
        keyManagerFactory.init(keyStore, null)

        return SSLContext.getInstance("TLSv1.3").apply {
            init(
                keyManagerFactory.keyManagers,
                arrayOf(PinnedDesktopTrustManager(expectedDesktopFingerprint)),
                SecureRandom(),
            )
        }
    }

    /**
     * Opens the desktop's TLS listener endpoint after mDNS resolution.
     * The fixed DNS name is used for SNI; certificate identity is established
     * by the pin and pairing flow, not by the network address.
     */
    fun connect(
        address: InetAddress,
        port: Int,
        expectedDesktopFingerprint: ByteArray?,
        connectTimeoutMillis: Int = 5000,
        onSocketCreated: (SSLSocket) -> Unit = {},
    ): SSLSocket {
        val rawSocket = Socket()
        try {
            rawSocket.connect(InetSocketAddress(address, port), connectTimeoutMillis)
            val tlsSocket = createClientContext(expectedDesktopFingerprint)
                .socketFactory
                .createSocket(rawSocket, "wipad.local", port, true) as SSLSocket
            tlsSocket.enabledProtocols = arrayOf("TLSv1.3")
            tlsSocket.soTimeout = 10_000
            onSocketCreated(tlsSocket)
            tlsSocket.startHandshake()
            return tlsSocket
        } catch (error: Exception) {
            runCatching { rawSocket.close() }
            throw error
        }
    }
}

/** Fingerprint and pairing-string functions shared with the Rust implementation. */
internal object TlsFingerprint {
    private val PAIRING_DOMAIN = "WiPad TLS peer comparison v1\u0000".toByteArray(Charsets.UTF_8)

    fun certificateFingerprint(certificateDer: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(certificateDer)

    /**
     * The phone certificate always comes first, followed by the desktop
     * certificate. Both devices therefore derive exactly the same string.
     */
    fun pairingString(phoneCertificateDer: ByteArray, desktopCertificateDer: ByteArray): String {
        require(phoneCertificateDer.isNotEmpty() && desktopCertificateDer.isNotEmpty()) {
            "Pairing certificates must not be empty"
        }

        val phoneFingerprint = certificateFingerprint(phoneCertificateDer)
        val desktopFingerprint = certificateFingerprint(desktopCertificateDer)
        val digest = MessageDigest.getInstance("SHA-256").run {
            update(PAIRING_DOMAIN)
            update(phoneFingerprint)
            update(desktopFingerprint)
            digest()
        }

        return buildString(14) {
            for (index in 0 until 6) {
                if (index == 2 || index == 4) append('-')
                append("%02X".format(java.util.Locale.ROOT, digest[index].toInt() and 0xFF))
            }
        }
    }

    fun matchesFingerprint(expected: ByteArray?, observed: ByteArray): Boolean {
        if (expected == null || expected.size != 32 || observed.size != 32) return false
        return MessageDigest.isEqual(expected, observed)
    }
}

/**
 * Persists only the desktop's public certificate fingerprint.
 * It is not secret material; SharedPreferences is sufficient for this pin.
 */
class TlsPeerPinStore(context: Context) {
    companion object {
        private const val PREFS_NAME = "wipad_tls"
        private const val DESKTOP_FINGERPRINT = "desktop_certificate_sha256"
    }

    private val preferences = context.applicationContext
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun loadDesktopFingerprint(): ByteArray? {
        val encoded = preferences.getString(DESKTOP_FINGERPRINT, null) ?: return null
        val decoded = try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (error: IllegalArgumentException) {
            throw IllegalStateException("Stored WiPad desktop pin is malformed", error)
        }
        if (decoded.size != 32) {
            throw IllegalStateException("Stored WiPad desktop pin has an invalid length")
        }
        return decoded
    }

    /** Call only after both peers approved and SESSION_READY was exchanged. */
    fun approveDesktop(certificate: X509Certificate) {
        val fingerprint = TlsFingerprint.certificateFingerprint(certificate.encoded)
        val saved = preferences.edit()
            .putString(DESKTOP_FINGERPRINT, Base64.encodeToString(fingerprint, Base64.NO_WRAP))
            .commit()
        check(saved) { "Could not persist the WiPad desktop trust pin" }
    }

    fun forgetDesktop() {
        preferences.edit().remove(DESKTOP_FINGERPRINT).apply()
    }
}

private class PinnedDesktopTrustManager(
    expectedFingerprint: ByteArray?,
) : X509TrustManager {
    private val expectedFingerprint = expectedFingerprint?.copyOf()

    override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String) {
        if (chain.size != 1) {
            throw CertificateException("WiPad expects one desktop identity certificate")
        }

        val certificate = chain[0]
        certificate.checkValidity()

        val publicKey = certificate.publicKey as? ECPublicKey
            ?: throw CertificateException("Desktop identity is not an EC certificate")
        if (publicKey.params.curve.field.fieldSize != 256) {
            throw CertificateException("Desktop identity is not ECDSA P-256")
        }

        val observed = TlsFingerprint.certificateFingerprint(certificate.encoded)
        if (expectedFingerprint != null
            && !TlsFingerprint.matchesFingerprint(expectedFingerprint, observed)
        ) {
            throw CertificateException("Desktop certificate changed; pairing approval is required")
        }
        // JSSE performs TLS 1.3 CertificateVerify signature verification after this
        // trust-manager decision. A null pin is therefore TOFU pairing, not
        // acceptance of unauthenticated TLS traffic.
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String) {
        throw CertificateException("WiPad Android client does not accept inbound TLS clients")
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}
