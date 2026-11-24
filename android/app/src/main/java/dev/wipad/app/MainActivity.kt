package dev.wipad.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

private val WiPadColors = darkColorScheme(
    primary = Color(0xFF64B5F6),
    secondary = Color(0xFF4DD0E1),
    background = Color(0xFF0D1B2A),
    surface = Color(0xFF14283B),
)

class MainActivity : ComponentActivity() {
    private var desktopStatus by mutableStateOf("Starting LAN discovery…")
    private var discovery: LanDiscovery? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(colorScheme = WiPadColors) {
                WiPadHome(desktopStatus)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        discovery = LanDiscovery(this) { status ->
            runOnUiThread { desktopStatus = status }
        }.also { it.start() }
    }

    override fun onStop() {
        discovery?.close()
        discovery = null
        super.onStop()
    }
}

@androidx.compose.runtime.Composable
private fun WiPadHome(desktopStatus: String) {
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("WiPad", style = MaterialTheme.typography.headlineLarge)
            Spacer(Modifier.height(12.dp))
            Text(
                "Your phone is ready to become a controller bridge.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onBackground,
            )
            Spacer(Modifier.height(28.dp))
            Text(
                "Bluetooth controller: Not connected",
                color = MaterialTheme.colorScheme.secondary,
            )
            Text(
                "Desktop: $desktopStatus",
                color = MaterialTheme.colorScheme.onBackground,
            )
        }
    }
}
