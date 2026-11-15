package dev.derust.tasmota.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

// Status colors, the same as the desktop app and the web page.
val Green = Color(0xFF7CC042)
val Yellow = Color(0xFFF2C21B)
val Red = Color(0xFFE04B3C)
val Orange = Color(0xFFF08C1E)
val OnStatus = Color(0xFF111111)

fun rssiColor(rssi: Long): Color = when {
    rssi >= 85 -> Green
    rssi >= 50 -> Yellow
    else -> Red
}

/** Material You colors from the wallpaper (minSdk 33 always has them). */
@Composable
fun TasmotaDerustTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    MaterialTheme(
        colorScheme = if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context),
        typography = Typography,
        content = content,
    )
}
