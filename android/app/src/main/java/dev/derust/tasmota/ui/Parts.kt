package dev.derust.tasmota.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.derust.tasmota.core.Device
import dev.derust.tasmota.isOffline
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.OnStatus
import dev.derust.tasmota.ui.theme.Red
import dev.derust.tasmota.ui.theme.rssiColor

/** Four signal bars from RSSI, or a red ✕ when the device is offline. */
@Composable
fun SignalBars(d: Device, modifier: Modifier = Modifier) {
    if (d.isOffline) {
        Box(
            modifier.size(16.dp).background(Red, CircleShape),
            contentAlignment = Alignment.Center,
        ) { Text("✕", color = Color.White, fontSize = 10.sp, fontWeight = FontWeight.Bold) }
        return
    }
    val r = d.rssi
    val level = when {
        r == null -> 0
        r >= 75 -> 4
        r >= 50 -> 3
        r >= 25 -> 2
        else -> 1
    }
    val on = r?.let { rssiColor(it) } ?: Color.Gray
    val off = MaterialTheme.colorScheme.outlineVariant
    Canvas(modifier.size(18.dp, 15.dp)) {
        val w = size.width / 4.6f
        for (i in 0 until 4) {
            val h = size.height * (0.3f + i * 0.233f)
            drawRoundRect(
                color = if (i < level) on else off,
                topLeft = Offset(i * w * 1.2f, size.height - h),
                size = Size(w, h),
                cornerRadius = CornerRadius(1.5f, 1.5f),
            )
        }
    }
}

/** Small colored box with a number (RSSI value, AP 1/2). */
@Composable
fun Chip(text: String, color: Color, minWidth: Dp = 20.dp) {
    Box(
        Modifier.defaultMinSize(minWidth = minWidth).background(color, RoundedCornerShape(4.dp))
            .padding(horizontal = 5.dp, vertical = 1.dp),
        contentAlignment = Alignment.Center,
    ) { Text(text, color = OnStatus, fontSize = 12.sp, fontWeight = FontWeight.SemiBold) }
}

@Composable
fun RssiChip(d: Device) {
    val r = d.rssi ?: return
    if (d.isOffline) return
    Chip("$r", rssiColor(r), minWidth = 34.dp)
}

/** AP chips: the network the device is on in green, the other one in red. */
@Composable
fun ApChips(d: Device, twoSsids: Boolean) {
    val ap = d.ap ?: return
    if (d.isOffline) return
    Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        for (k in 1..(if (twoSsids) 2 else 1)) Chip("$k", if (k.toLong() == ap) Green else Red)
    }
}

/**
 * Relay buttons. A single relay shows ON/OFF, several show their number.
 * Disabled when the device is offline or the app is read-only.
 */
@Composable
fun RelayButtons(
    d: Device,
    enabled: Boolean,
    large: Boolean,
    onToggle: (Int) -> Unit,
) {
    val single = d.power.size == 1
    Row(horizontalArrangement = Arrangement.spacedBy(if (large) 10.dp else 6.dp)) {
        d.power.forEachIndexed { i, p ->
            val label = if (single) when (p) { true -> "ON"; false -> "OFF"; null -> "?" } else "${i + 1}"
            val mod = Modifier.height(if (large) 48.dp else 34.dp)
                .defaultMinSize(minWidth = if (large) 88.dp else 50.dp)
            val pad = PaddingValues(horizontal = if (large) 16.dp else 8.dp)
            val shape = RoundedCornerShape(if (large) 12.dp else 9.dp)
            if (p == true) {
                Button(
                    onClick = { onToggle(i + 1) }, enabled = enabled, modifier = mod, contentPadding = pad, shape = shape,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = Green, contentColor = OnStatus,
                        disabledContainerColor = Green.copy(alpha = 0.45f), disabledContentColor = OnStatus.copy(alpha = 0.7f),
                    ),
                ) { Text(label, fontWeight = FontWeight.SemiBold) }
            } else {
                FilledTonalButton(
                    onClick = { onToggle(i + 1) }, enabled = enabled, modifier = mod, contentPadding = pad, shape = shape,
                ) { Text(label, fontWeight = FontWeight.SemiBold) }
            }
        }
    }
}

/** Square console button (the device sheet puts it beside the name and status lines). */
@Composable
fun ConsoleButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(8.dp)
    Box(
        modifier
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.6f), shape)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, shape)
            .clickable(onClick = onClick)
            .semantics { contentDescription = "Console" },
        contentAlignment = Alignment.Center,
    ) {
        // Icon and label inside the square.
        androidx.compose.foundation.layout.Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(Icons.Console, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                "CLI", fontSize = 10.sp, lineHeight = 11.sp, fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
