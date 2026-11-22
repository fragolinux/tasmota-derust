package dev.derust.tasmota.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.derust.tasmota.core.Device
import dev.derust.tasmota.core.MqttDefaults
import dev.derust.tasmota.core.OtaDefault
import dev.derust.tasmota.core.OtaDevice
import dev.derust.tasmota.core.Search
import dev.derust.tasmota.fmtShort
import dev.derust.tasmota.isOffline
import dev.derust.tasmota.offlineLabel
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.OnStatus
import dev.derust.tasmota.ui.theme.Orange

/** "HTTP": reached over HTTP only, not over MQTT. */
@Composable
fun HttpChip() {
    Text(
        "HTTP", fontSize = 11.sp, fontWeight = FontWeight.Bold, color = OnStatus,
        modifier = Modifier.background(Orange, RoundedCornerShape(4.dp)).padding(horizontal = 5.dp, vertical = 1.dp),
    )
}

private val NewTint = Color(0xFFF2C21B).copy(alpha = 0.22f)

/**
 * The dashboard: one card per device with big relay buttons, the light colour and its main
 * values. (TODO: let the user choose the values shown on the cards.)
 */
@Composable
fun Dashboard(
    devices: List<Device>,
    selecting: Boolean,
    selected: Set<String>,
    contentPadding: PaddingValues,
    onOpen: (Device) -> Unit,
    onToggle: (Device, Int) -> Unit,
    onLight: (Device) -> Unit,
    onSelect: (Device, Boolean) -> Unit,
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(160.dp),
        contentPadding = PaddingValues(start = 10.dp, end = 10.dp, top = 6.dp, bottom = contentPadding.calculateBottomPadding() + 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.fillMaxSize(),
    ) {
        items(devices, key = { it.topic }) { d ->
            val online = d.online == true
            Surface(
                shape = RoundedCornerShape(14.dp),
                color = if (d.new) NewTint else MaterialTheme.colorScheme.surfaceContainer,
                modifier = Modifier.fillMaxWidth().alpha(if (d.isOffline) 0.55f else 1f).clickable { onOpen(d) },
            ) {
                Column(Modifier.padding(10.dp).defaultMinSize(minHeight = 104.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (selecting) Checkbox(
                            checked = d.topic in selected, onCheckedChange = { onSelect(d, it) },
                            modifier = Modifier.size(28.dp),
                        )
                        SignalBars(d)
                        Spacer(Modifier.width(6.dp))
                        if (d.http != null) { HttpChip(); Spacer(Modifier.width(4.dp)) }
                        Text(
                            (if (d.new) "★ " else "") + d.name, fontWeight = FontWeight.SemiBold, fontSize = 15.sp,
                            maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Text(
                        if (d.isOffline) d.offlineLabel else d.telemetry ?: d.uptime?.let { "up " + fmtShort(it) } ?: "",
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.weight(1f, fill = false))
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                        d.light?.let { l ->
                            Box(
                                Modifier.size(42.dp).background(swatchColor(l.swatch), RoundedCornerShape(10.dp))
                                    .clickable(enabled = online) { onLight(d) },
                            )
                        }
                        val single = d.power.size == 1
                        d.power.forEachIndexed { i, p ->
                            val label = if (single) when (p) { true -> "ON"; false -> "OFF"; null -> "?" } else "${i + 1}"
                            val mod = Modifier.weight(1f).height(42.dp)
                            val pad = PaddingValues(horizontal = 2.dp)
                            val shape = RoundedCornerShape(10.dp)
                            if (p == true) Button(
                                onClick = { onToggle(d, i + 1) }, enabled = online, modifier = mod, contentPadding = pad, shape = shape,
                                colors = ButtonDefaults.buttonColors(containerColor = Green, contentColor = OnStatus),
                            ) { Text(label, fontWeight = FontWeight.SemiBold) }
                            else FilledTonalButton(
                                onClick = { onToggle(d, i + 1) }, enabled = online, modifier = mod, contentPadding = pad, shape = shape,
                            ) { Text(label, fontWeight = FontWeight.SemiBold) }
                        }
                    }
                }
            }
        }
    }
}

/** Search the local network over HTTP. */
@Composable
fun FindDialog(search: Search?, localRange: String?, onSearch: (String, String) -> Unit, onDismiss: () -> Unit) {
    // The phone's own network (address and netmask).
    var range by remember { mutableStateOf(localRange ?: "") }
    var password by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Find devices on the network") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "Asks every address of the network for its status over HTTP (read-only), to find Tasmota devices, " +
                        "also those without MQTT. Those not on MQTT are added to the list, marked HTTP. " +
                        "Only devices that answer as Tasmota are added: Shelly, routers and other web servers are left out.",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedTextField(
                    range, { range = it }, singleLine = true, label = { Text("Addresses") },
                    placeholder = { Text("192.168.1.0/24") }, modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    password, { password = it }, singleLine = true, label = { Text("Web password (if any)") },
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth(),
                )
                if (search != null) {
                    LinearProgressIndicator(
                        progress = { if (search.total == 0u) 0f else search.done.toFloat() / search.total.toFloat() },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        when {
                            search.running -> "${search.done} / ${search.total}"
                            search.tasmota == 0u -> "No Tasmota answered."
                            search.added == 0u -> "${search.tasmota} Tasmota found, all already on MQTT."
                            else -> "${search.tasmota} Tasmota found; ${search.added} reached over HTTP only, added to the list."
                        },
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onSearch(range.trim(), password) }, enabled = search?.running != true) { Text("Search") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

/** Upgrade the ticked devices: what each would do; the skipped ones say why. */
@Composable
fun UpgradeManyDialog(devices: List<Device>, onUpgrade: (List<String>) -> Unit, onDismiss: () -> Unit) {
    val rows = devices.map { d ->
        val plan = d.upgradePlan
        val ok = d.online == true && plan.kind in setOf("step", "final") && !(plan.kind == "final" && d.upToDate)
        val why = when {
            d.online != true -> "offline"
            plan.kind == "final" && d.upToDate -> "up to date"
            else -> plan.title
        }
        Triple(d, ok, why)
    }
    val start = rows.filter { it.second }.map { it.first.topic }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (start.isEmpty()) "Nothing to upgrade" else "Upgrade ${start.size} device${if (start.size > 1) "s" else ""}?") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    "Each device upgrades on its own, at the same time; a failure stops only that device. " +
                        "Old firmware goes through the upgrade path, step after step, by itself.",
                    style = MaterialTheme.typography.bodySmall,
                )
                for ((d, ok, why) in rows) {
                    Text(d.name, fontWeight = FontWeight.SemiBold)
                    Text(
                        (if (ok) "" else "skipped: ") + why.replace("->", "→"),
                        style = MaterialTheme.typography.bodySmall,
                        color = if (ok) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text("Do not power the devices off during the upgrade.", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            if (start.isNotEmpty()) TextButton(onClick = { onUpgrade(start); onDismiss() }) { Text("Upgrade") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(if (start.isEmpty()) "OK" else "Cancel") } },
    )
}

/** Connect HTTP-only devices to the broker. `onSend(host, port, user, password or null)`. */
@Composable
fun EnableMqttDialog(
    devices: List<Device>,
    defaults: MqttDefaults?,
    onSend: (String, Int, String, String?) -> Unit,
    onDismiss: () -> Unit,
) {
    var host by remember { mutableStateOf(defaults?.host ?: "") }
    var port by remember { mutableStateOf((defaults?.port ?: 1883u).toString()) }
    var user by remember { mutableStateOf(defaults?.user ?: "") }
    var password by remember { mutableStateOf("") }
    val portNum = port.toIntOrNull()?.takeIf { it in 1..65535 }
    val loopback = host.trim() in setOf("127.0.0.1", "localhost", "::1")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Enable MQTT") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Connect to the broker: ${devices.joinToString(", ") { it.name }}.")
                Text(
                    "They get MQTT on (SetOption3 1), broker, port, user and password in one Backlog over HTTP, " +
                        "restart, and show up over MQTT.",
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedTextField(host, { host = it }, singleLine = true, label = { Text("Broker") }, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(
                    port, { port = it }, singleLine = true, label = { Text("Port") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(user, { user = it }, singleLine = true, label = { Text("User") }, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(
                    password, { password = it }, singleLine = true, label = { Text("Password") },
                    placeholder = { if (defaults?.hasPassword == true) Text("empty: this app's broker password") },
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth(),
                )
                if (loopback) Text(
                    "${host.trim()} is this phone: the devices need the broker's network address.",
                    color = Orange, style = MaterialTheme.typography.bodySmall,
                )
                Text("The password travels over plain HTTP, like Tasmota's own web page.", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onSend(host.trim(), portNum ?: 1883, user.trim(), password.ifEmpty { null }); onDismiss() },
                enabled = host.isNotBlank() && portNum != null,
            ) { Text("Send") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** One device's firmware server and highest version. */
@Composable
fun OtaDeviceDialog(
    name: String,
    current: OtaDevice,
    defaults: OtaDefault,
    onSave: (OtaDevice) -> String?,
    onDismiss: () -> Unit,
) {
    var ownUrl by remember { mutableStateOf(current.customUrl != null) }
    var url by remember { mutableStateOf(current.customUrl ?: "") }
    var ownMax by remember { mutableStateOf(current.maxVersion != null) }
    var max by remember { mutableStateOf(current.maxVersion ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("$name: firmware source") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(ownUrl, { ownUrl = it })
                    Text("Own firmware server")
                }
                if (ownUrl) OutlinedTextField(
                    url, { url = it }, singleLine = true, placeholder = { Text("http://server/{build}.bin.gz") },
                    supportingText = { Text("Empty: the official server") }, modifier = Modifier.fillMaxWidth(),
                ) else Text("Default: ${defaults.customUrl.ifEmpty { "official server" }}", style = MaterialTheme.typography.bodySmall)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(ownMax, { ownMax = it })
                    Text("Own highest version")
                }
                if (ownMax) OutlinedTextField(
                    max, { max = it }, singleLine = true, placeholder = { Text("e.g. 11.1.0") },
                    supportingText = { Text("Empty: the latest") }, modifier = Modifier.fillMaxWidth(),
                ) else Text("Default: ${defaults.maxVersion.ifEmpty { "latest" }}", style = MaterialTheme.typography.bodySmall)
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                error = onSave(OtaDevice(if (ownUrl) url else null, if (ownMax) max else null))
                if (error == null) onDismiss()
            }) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** The web password of an HTTP-only device (kept only while the app runs). */
@Composable
fun HttpPasswordDialog(address: String, onOk: (String) -> Unit, onDismiss: () -> Unit) {
    var pw by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Web password") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("The web password of $address (kept only while the app runs).")
                OutlinedTextField(
                    pw, { pw = it }, singleLine = true, visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = { TextButton(onClick = { onOk(pw); onDismiss() }) { Text("OK") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
