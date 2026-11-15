package dev.derust.tasmota.ui

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import dev.derust.tasmota.core.Device
import dev.derust.tasmota.core.Snapshot
import dev.derust.tasmota.fmtDuration
import dev.derust.tasmota.isOffline
import dev.derust.tasmota.offlineLabel
import dev.derust.tasmota.needsUpgradePath
import dev.derust.tasmota.upgradeBusy
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.Orange
import dev.derust.tasmota.ui.theme.Red

private sealed interface Confirm {
    data class Ap(val to: Int) : Confirm
    data object Upgrade : Confirm
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun DeviceSheet(
    d: Device,
    s: Snapshot,
    onDismiss: () -> Unit,
    onToggle: (Int) -> Unit,
    onSwitchAp: (Int) -> Unit,
    onUpgrade: () -> Unit,
    /** Read-only mode (app-wide). Commands stay tappable and explain why nothing happens. */
    readOnly: Boolean,
    onReadOnly: () -> Unit,
) {
    val context = LocalContext.current
    var confirm by remember { mutableStateOf<Confirm?>(null) }
    val canCommand = d.online == true
    fun ask(c: Confirm) = if (readOnly) onReadOnly() else confirm = c

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp).padding(bottom = 24.dp).navigationBarsPadding(),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SignalBars(d)
                Spacer(Modifier.width(10.dp))
                Text(
                    d.name, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
                )
            }
            Text(
                listOfNotNull(
                    when (d.online) { true -> "Online"; false -> d.offlineLabel; null -> "Waiting for an answer…" },
                    d.module,
                ).joinToString(" · "),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            if (d.power.isNotEmpty()) {
                Spacer(Modifier.height(16.dp))
                RelayButtons(d, enabled = canCommand, large = true, onToggle = onToggle)
            }

            Spacer(Modifier.height(14.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val url = d.webUrl
                if (!d.isOffline && url != null) {
                    OutlinedButton(onClick = { context.startActivity(Intent(Intent.ACTION_VIEW, url.toUri())) }) {
                        Icon(Icons.External, null, Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Web UI")
                    }
                }
                val ap = d.ap
                if (s.ssid2 != null && ap != null && !d.isOffline) {
                    val other = if (ap == 1L) 2 else 1
                    OutlinedButton(onClick = { ask(Confirm.Ap(other)) }, enabled = canCommand) {
                        Text("Switch to SSId $other")
                    }
                }
                if (!d.upgradeBusy) {
                    OutlinedButton(
                        onClick = { ask(Confirm.Upgrade) }, enabled = canCommand,
                        colors = if (d.outdated) ButtonDefaults.outlinedButtonColors(contentColor = Orange) else ButtonDefaults.outlinedButtonColors(),
                    ) { Text("Upgrade…") }
                }
            }
            d.upgradeState?.let { st ->
                Spacer(Modifier.height(10.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (d.upgradeBusy) {
                        CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                    }
                    Text(
                        d.upgradeText ?: "",
                        color = when (st) {
                            "done" -> Green
                            "failed" -> Red
                            "note" -> MaterialTheme.colorScheme.onSurfaceVariant
                            else -> MaterialTheme.colorScheme.onSurface
                        },
                    )
                }
            }
            if (readOnly) {
                Spacer(Modifier.height(8.dp))
                Text(
                    "Read-only is on: commands are not sent. Use the lock in the top bar to allow them.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            for ((title, rows) in sections(d, s)) {
                if (rows.isEmpty()) continue
                Text(
                    title.uppercase(), style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 20.dp, bottom = 6.dp),
                )
                Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(12.dp))) {
                    rows.forEachIndexed { i, (k, v) ->
                        if (i > 0) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                        Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp)) {
                            Text(k, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.width(12.dp))
                            Text(
                                v, textAlign = TextAlign.End, modifier = Modifier.weight(1f),
                                color = if (k == "Version" && d.outdated && !d.isOffline) Orange else MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                }
            }
        }
    }

    when (val c = confirm) {
        is Confirm.Ap -> {
            val from = d.ap?.toInt() ?: 0
            val name = { n: Int -> (if (n == 1) s.ssid1 else s.ssid2) ?: "?" }
            AlertDialog(
                onDismissRequest = { confirm = null },
                title = { Text("Switch ${d.name} to SSId ${c.to}?") },
                text = {
                    Text(
                        "Now connected to SSId $from (${name(from)}).\n\n" +
                            "It will restart and connect to SSId ${c.to} (${name(c.to)}).\n\n" +
                            "If it cannot connect there, Tasmota tries the other network again.",
                    )
                },
                confirmButton = { TextButton(onClick = { confirm = null; onSwitchAp(c.to) }) { Text("Restart and switch") } },
                dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
            )
        }
        Confirm.Upgrade -> AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text("Upgrade ${d.name}?") },
            text = {
                Column {
                    Text("Current: ${d.version ?: "unknown"}\nLatest release: ${s.latest ?: "unknown"}\nOtaUrl: ${d.otaUrl ?: "unknown"}")
                    Spacer(Modifier.height(10.dp))
                    Text("The device downloads the firmware from its OtaUrl and restarts. Do not power it off during the upgrade.")
                    if (d.needsUpgradePath) {
                        Spacer(Modifier.height(10.dp))
                        Text(
                            "This firmware is old. Tasmota needs intermediate upgrade steps for old versions; " +
                                "upgrading straight to the latest release can fail.",
                            color = Orange,
                        )
                    }
                    if (d.otaUrl != null && !d.otaUrl!!.contains("ota.tasmota.com")) {
                        Spacer(Modifier.height(10.dp))
                        Text("This device uses a custom OtaUrl.", color = Orange)
                    }
                }
            },
            confirmButton = { TextButton(onClick = { confirm = null; onUpgrade() }) { Text("Upgrade") } },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
        )
        null -> {}
    }
}

private fun sections(d: Device, s: Snapshot): List<Pair<String, List<Pair<String, String>>>> {
    fun rows(vararg p: Pair<String, Any?>) = p.mapNotNull { (k, v) -> v?.toString()?.takeIf { it.isNotEmpty() }?.let { k to it } }
    val apName = d.ap?.let { ap -> "SSId $ap" + ((if (ap == 1L) s.ssid1 else s.ssid2)?.let { " ($it)" } ?: "") }
    return listOf(
        "Status" to rows(
            "Module" to d.module, "Uptime" to d.uptime?.let { fmtDuration(it) }, "Telemetry" to d.telemetry,
            "RestartReason" to d.restartReason, "BootCount" to d.bootCount, "LoadAvg" to d.loadAvg,
            "Sleep" to d.sleep, "MqttCount" to d.mqttCount,
        ),
        "Firmware" to rows(
            "Version" to d.version, "Core" to d.core, "SDK" to d.sdk, "ProgramSize" to d.programSize,
            "Free" to d.free, "OtaUrl" to d.otaUrl,
        ),
        "Network" to rows(
            "IPAddress" to d.ip, "Hostname" to d.hostname, "Mac" to d.mac, "Gateway" to d.gateway,
            "SSId" to d.ssid, "AP" to apName, "BSSId" to d.bssid, "Channel" to d.channel, "RSSI" to d.rssi,
            "LinkCount" to d.linkCount, "Downtime" to d.downtime?.let { fmtDuration(it) },
        ),
        "MQTT" to rows(
            "Topic" to d.topic, "FullTopic" to d.fullTopic, "CommandTopic" to d.cmndTopic,
            "StatTopic" to d.statTopic, "TeleTopic" to d.teleTopic, "FallbackTopic" to d.fallbackTopic,
            "GroupTopic" to d.groupTopic,
        ),
    )
}
