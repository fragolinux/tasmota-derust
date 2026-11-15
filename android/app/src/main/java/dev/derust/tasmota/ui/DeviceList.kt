package dev.derust.tasmota.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.derust.tasmota.View
import dev.derust.tasmota.core.Device
import dev.derust.tasmota.core.Snapshot
import dev.derust.tasmota.fmtShort
import dev.derust.tasmota.isOffline
import dev.derust.tasmota.offlineLabel
import dev.derust.tasmota.upgradeBusy
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.Orange
import dev.derust.tasmota.ui.theme.Red

@Composable
fun DeviceList(
    snapshot: Snapshot,
    devices: List<Device>,
    view: View,
    contentPadding: PaddingValues,
    onOpen: (Device) -> Unit,
    onToggle: (Device, Int) -> Unit,
) {
    LazyColumn(contentPadding = contentPadding, modifier = Modifier.fillMaxWidth()) {
        viewInfo(snapshot, view)?.let { info ->
            item(key = "info") {
                Text(
                    info, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                )
            }
        }
        items(devices, key = { it.topic }) { d ->
            DeviceRow(d, view, snapshot, onOpen = { onOpen(d) }, onToggle = { n -> onToggle(d, n) })
            HorizontalDivider(Modifier.padding(start = 46.dp), color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
        }
    }
}

/** A line above the list: the two Wi-Fi networks, or the latest Tasmota release. */
private fun viewInfo(s: Snapshot, view: View): String? = when (view) {
    View.Wifi -> listOfNotNull(s.ssid1?.let { "1: $it" }, s.ssid2?.let { "2: $it" }).joinToString("   ").ifEmpty { null }
    View.Firmware -> s.latest?.let { "Latest release: $it" + (s.latestDev?.let { d -> " · development: $d" } ?: "") }
        ?: if (s.latestError != null) "Latest release unknown" else "Checking latest release…"
    else -> null
}

@Composable
private fun DeviceRow(d: Device, view: View, s: Snapshot, onOpen: () -> Unit, onToggle: (Int) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onOpen)
            .alpha(if (d.isOffline) 0.55f else 1f)
            .defaultMinSize(minHeight = 58.dp)
            .padding(start = 16.dp, end = 12.dp, top = 7.dp, bottom = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SignalBars(d)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                d.name, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            val sub = subtitle(d, view)
            if (sub.isNotEmpty()) Text(
                sub, style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
                fontFamily = if (view == View.Mqtt && !d.isOffline) FontFamily.Monospace else null,
            )
        }
        Spacer(Modifier.width(8.dp))
        if (!d.isOffline) Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            RowEnd(d, view, s, onToggle)
        }
    }
}

private fun subtitle(d: Device, view: View): String {
    if (d.isOffline) return listOfNotNull(d.offlineLabel, d.module).joinToString(" · ")
    return when (view) {
        View.Home -> listOfNotNull(d.module, d.telemetry ?: d.uptime?.let { "up " + fmtShort(it) })
        View.Health -> listOfNotNull(d.restartReason, d.bootCount?.let { "$it boots" })
        View.Firmware -> listOfNotNull(d.module, d.core?.let { "core $it" })
        View.Wifi -> listOfNotNull(d.ip, d.ssid)
        View.Mqtt -> listOf(d.fullTopic)
    }.joinToString(" · ")
}

@Composable
private fun RowEnd(d: Device, view: View, s: Snapshot, onToggle: (Int) -> Unit) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    when (view) {
        View.Home -> {
            d.light?.let {
                Box(Modifier.size(18.dp).background(swatchColor(it.swatch), RoundedCornerShape(4.dp)))
            }
            RelayButtons(d, enabled = d.online == true, large = false, onToggle = onToggle)
        }
        View.Health -> {
            Text(fmtShort(d.uptime), color = muted, fontSize = 13.sp)
            RssiChip(d)
        }
        View.Firmware -> {
            if (d.upgradeBusy) CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
            else when (d.upgradeState) {
                "done" -> Text("✓", color = Green)
                "failed" -> Text("!", color = Red, fontWeight = FontWeight.Bold)
            }
            d.versionShort?.let {
                Text(it, fontSize = 13.sp, color = if (d.outdated) Orange else muted,
                    fontWeight = if (d.outdated) FontWeight.Bold else null)
            }
        }
        View.Wifi -> {
            ApChips(d, twoSsids = s.ssid2 != null)
            RssiChip(d)
        }
        View.Mqtt -> Text(d.topic, color = muted, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
    }
}
