package dev.derust.tasmota

import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.LaunchedEffect
import dev.derust.tasmota.ui.App
import dev.derust.tasmota.ui.theme.TasmotaDerustTheme

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            TasmotaDerustTheme {
                // Android 17+ asks the user before an app can reach devices on the local network.
                if (Build.VERSION.SDK_INT >= 37) {
                    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
                    LaunchedEffect(Unit) { ask.launch("android.permission.ACCESS_LOCAL_NETWORK") }
                }
                App(vm)
            }
        }
    }
}
