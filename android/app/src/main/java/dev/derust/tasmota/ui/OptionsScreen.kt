package dev.derust.tasmota.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.derust.tasmota.core.Options
import dev.derust.tasmota.core.SetOption
import dev.derust.tasmota.ui.theme.Orange
import dev.derust.tasmota.ui.theme.Red

private fun SetOption.valueText(v: Long) = when {
    numeric -> v.toString()
    v == 1L -> "ON (1)"
    else -> "OFF (0)"
}

private fun SetOption.matches(q: String): Boolean {
    if (q.isBlank()) return true
    val hay = "so$n setoption$n $title $category ${details.joinToString(" ")}".lowercase()
    // A number ("4", "so4", "setoption4") matches that option only, not SO40–49.
    val number = Regex("^(?:setoption|so)?(\\d+)$")
    return q.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }.all { t ->
        number.find(t)?.let { it.groupValues[1].toIntOrNull() == n.toInt() } ?: hay.contains(t)
    }
}

/**
 * The SetOptions of one device, full screen above the device sheet: grouped by category,
 * with search, switches and number fields. Risky options ask first; MQTT or Wi-Fi off is
 * refused (also by the Rust core).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OptionsScreen(
    name: String,
    online: Boolean,
    readOnly: Boolean,
    opts: Options?,
    onClose: () -> Unit,
    onSet: (Int, Long) -> Unit,
    onReadOnly: () -> Unit,
) {
    var search by rememberSaveable { mutableStateOf("") }
    var onlyChanged by rememberSaveable { mutableStateOf(false) }
    var openCats by remember { mutableStateOf(setOf("General")) }
    var confirm by remember { mutableStateOf<Pair<SetOption, Long>?>(null) }
    var blocked by remember { mutableStateOf<String?>(null) }
    val uri = LocalUriHandler.current

    fun request(o: SetOption, v: Long) {
        when {
            o.blockedValue?.toLong() == v && o.blockedReason != null -> blocked = o.blockedReason
            readOnly -> onReadOnly()
            o.risk != "normal" -> confirm = o to v
            else -> onSet(o.n.toInt(), v)
        }
    }

    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("$name — SetOptions", maxLines = 1) },
                    navigationIcon = { IconButton(onClick = onClose) { Icon(Icons.Back, "Back") } },
                )
            },
        ) { pad ->
            LazyColumn(
                Modifier.fillMaxSize().imePadding(),
                contentPadding = PaddingValues(
                    start = 16.dp, end = 16.dp, top = pad.calculateTopPadding(), bottom = pad.calculateBottomPadding() + 24.dp,
                ),
            ) {
                item {
                    OutlinedTextField(
                        value = search, onValueChange = { search = it }, singleLine = true,
                        placeholder = { Text("Search (number or words)") },
                        leadingIcon = { Icon(Icons.Search, null, Modifier.size(18.dp)) },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        FilterChip(selected = onlyChanged, onClick = { onlyChanged = !onlyChanged }, label = { Text("Only changed") })
                    }
                    if (onlyChanged) {
                        Text(
                            "Compared with a freshly flashed current Tasmota: devices upgraded from old versions can keep older values.",
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    val muted = MaterialTheme.colorScheme.onSurfaceVariant
                    if (opts == null) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(8.dp))
                            Text("Loading…", color = muted)
                        }
                    } else {
                        if (opts.texts == "loading" && opts.options.all { it.title.startsWith("SetOption") }) {
                            Text("Loading the descriptions from the Tasmota documentation…", color = muted)
                        }
                        if (opts.texts == "failed") {
                            Text("Descriptions not available (no internet?): options are shown by number.", color = Orange)
                        }
                        if (!opts.reported) Text("The device has not reported its options yet.", color = muted)
                    }
                    if (readOnly) {
                        Text("Read-only is on: options can't be changed.", color = Orange, style = MaterialTheme.typography.bodySmall)
                    } else if (!online) {
                        Text("The device is not online.", color = Red, style = MaterialTheme.typography.bodySmall)
                    }
                    Spacer(Modifier.height(8.dp))
                }
                val all = opts?.options.orEmpty()
                val shown = all.filter { (!onlyChanged || !it.isDefault) && it.matches(search) }
                val filtering = search.isNotBlank() || onlyChanged
                if (opts != null && opts.reported && shown.isEmpty()) {
                    item { Text("No option matches.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
                for (cat in all.map { it.category }.distinct()) {
                    val list = shown.filter { it.category == cat }
                    if (list.isEmpty()) continue
                    val expanded = filtering || cat in openCats
                    item(key = "cat-$cat") {
                        Row(
                            Modifier.fillMaxWidth()
                                .clickable { openCats = if (cat in openCats) openCats - cat else openCats + cat }
                                .padding(vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(cat, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                            Text("${list.size}  " + if (expanded) "▾" else "▸", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        HorizontalDivider()
                    }
                    if (expanded) {
                        items(list, key = { "so-${it.n}" }) { o ->
                            OptionRow(o, enabled = online, onRequest = { v -> request(o, v) }, onLink = { uri.openUri(o.link) })
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                        }
                    }
                }
                if (opts != null && opts.reported) {
                    item {
                        TextButton(onClick = { uri.openUri(opts.docs) }) { Text("All SetOptions in the Tasmota documentation") }
                    }
                }
            }
        }

        confirm?.let { (o, v) ->
            AlertDialog(
                onDismissRequest = { confirm = null },
                title = { Text("Change SetOption${o.n} on $name?") },
                text = {
                    Column {
                        Text(o.title, fontWeight = FontWeight.Bold)
                        o.details.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                        Spacer(Modifier.height(8.dp))
                        Text("From ${o.valueText(o.value.toLong())} to ${o.valueText(v)}.")
                        o.warning?.let {
                            Spacer(Modifier.height(8.dp))
                            Text(it, color = if (o.risk == "danger") Red else Orange)
                        }
                    }
                },
                confirmButton = { TextButton(onClick = { confirm = null; onSet(o.n.toInt(), v) }) { Text("Change") } },
                dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
            )
        }
        blocked?.let { why ->
            AlertDialog(
                onDismissRequest = { blocked = null },
                title = { Text("Not allowed from here") },
                text = { Text(why) },
                confirmButton = { TextButton(onClick = { blocked = null }) { Text("OK") } },
            )
        }
    }
}

@Composable
private fun OptionRow(o: SetOption, enabled: Boolean, onRequest: (Long) -> Unit, onLink: () -> Unit) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("SO${o.n}", fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelSmall, color = muted)
                    when (o.risk) {
                        "danger" -> Text("⚠", color = Red)
                        "app" -> Text("⚠", color = Orange)
                    }
                    if (!o.isDefault) {
                        Text("default ${o.defaultValue}", color = Orange, style = MaterialTheme.typography.labelSmall)
                    }
                }
                Text(o.title, style = MaterialTheme.typography.bodyMedium)
            }
            Spacer(Modifier.width(8.dp))
            if (o.numeric) {
                var text by remember(o.n, o.value) { mutableStateOf(o.value.toString()) }
                val parsed = text.toLongOrNull()
                val valid = parsed != null && parsed in minOf(o.min, o.value).toLong()..o.max.toLong()
                OutlinedTextField(
                    value = text, onValueChange = { text = it.filter(Char::isDigit).take(3) }, singleLine = true,
                    enabled = enabled, isError = !valid, modifier = Modifier.width(84.dp),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                )
                Spacer(Modifier.width(6.dp))
                OutlinedButton(
                    onClick = { parsed?.let(onRequest) },
                    enabled = enabled && valid && parsed != o.value.toLong(),
                ) { Text("Set") }
            } else {
                Switch(checked = o.value == 1u, onCheckedChange = { onRequest(if (it) 1L else 0L) }, enabled = enabled)
            }
        }
        if (o.details.isNotEmpty() || o.warning != null) {
            Spacer(Modifier.height(4.dp))
            o.details.forEach { Text(it, style = MaterialTheme.typography.bodySmall, color = muted) }
            o.warning?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = if (o.risk == "danger") Red else Orange) }
        }
        Text(
            "Documentation", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.clickable(onClick = onLink).padding(vertical = 4.dp),
        )
    }
}
