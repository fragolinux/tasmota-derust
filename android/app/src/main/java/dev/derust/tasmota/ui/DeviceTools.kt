package dev.derust.tasmota.ui

import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.derust.tasmota.MainViewModel
import dev.derust.tasmota.MainViewModel.FileResult
import dev.derust.tasmota.core.BackupFile
import dev.derust.tasmota.core.Device
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.Orange
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Whole-configuration backup and restore of one device, through its own web server (the same
 * .dmp file as its Configuration > Backup / Restore pages). Backup: the file is fetched, then
 * the system's "save as" picker writes it. Restore: the system picker opens a .dmp, a
 * confirmation follows. The web password is asked only when the device wants it, never stored.
 */
class DeviceFiles internal constructor(
    private val start: (Request) -> Unit,
) {
    internal sealed interface Request {
        val topic: String
        val name: String
        data class Backup(override val topic: String, override val name: String) : Request
        data class Restore(override val topic: String, override val name: String) : Request
    }

    fun backup(d: Device) = start(Request.Backup(d.topic, d.name))
    fun restore(d: Device) = start(Request.Restore(d.topic, d.name))
}

@Composable
fun rememberDeviceFiles(vm: MainViewModel): DeviceFiles {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val toast = { msg: String -> Toast.makeText(context, msg, Toast.LENGTH_SHORT).show() }
    // Errors stay on screen until read (a toast cuts long ones).
    var error by remember { mutableStateOf<String?>(null) }

    var pending by remember { mutableStateOf<DeviceFiles.Request?>(null) }
    // The fetched backup waiting for the "save as" picker.
    var toSave by remember { mutableStateOf<BackupFile?>(null) }
    // The file picked for a restore, waiting for the confirmation.
    var toRestore by remember { mutableStateOf<Pair<String, ByteArray>?>(null) }
    // Asking the web password: (wrong one given?) and what to retry with it.
    var askPassword by remember { mutableStateOf<Pair<Boolean, (String) -> Unit>?>(null) }

    val saveLauncher = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val file = toSave
        toSave = null
        if (uri == null || file == null) return@rememberLauncherForActivityResult
        scope.launch {
            val ok = withContext(Dispatchers.IO) {
                runCatching { context.contentResolver.openOutputStream(uri)?.use { it.write(file.data) } != null }.getOrDefault(false)
            }
            toast(if (ok) "Saved ${file.name}" else "Could not save the file")
        }
    }
    val openLauncher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) { pending = null; return@rememberLauncherForActivityResult }
        scope.launch {
            val data = withContext(Dispatchers.IO) {
                runCatching { context.contentResolver.openInputStream(uri)?.use { it.readBytes() } }.getOrNull()
            }
            if (data == null) { toast("Could not read the file"); pending = null; return@launch }
            val name = uri.lastPathSegment?.substringAfterLast('/') ?: "the file"
            toRestore = name to data
        }
    }

    fun runBackup(r: DeviceFiles.Request, password: String?) {
        scope.launch {
            toast("Backing up ${r.name}…")
            when (val res = vm.backup(r.topic, password)) {
                is FileResult.Ok -> { toSave = res.value; saveLauncher.launch(res.value.name) }
                is FileResult.NeedsPassword -> askPassword = res.wrong to { pw -> runBackup(r, pw) }
                is FileResult.Error -> error = res.message
            }
        }
    }

    fun runRestore(r: DeviceFiles.Request, data: ByteArray, password: String?) {
        scope.launch {
            toast("Restoring ${r.name}…")
            when (val res = vm.restore(r.topic, password, data)) {
                is FileResult.Ok -> toast("${r.name}: configuration restored, the device restarts")
                is FileResult.NeedsPassword -> askPassword = res.wrong to { pw -> runRestore(r, data, pw) }
                is FileResult.Error -> error = res.message
            }
        }
    }

    val files = remember {
        DeviceFiles { r ->
            pending = r
            when (r) {
                is DeviceFiles.Request.Backup -> runBackup(r, null)
                is DeviceFiles.Request.Restore ->
                    if (vm.readOnly) { vm.readOnlyHit(); pending = null } else openLauncher.launch(arrayOf("*/*"))
            }
        }
    }

    val r = pending
    toRestore?.let { (fileName, data) ->
        if (r == null) { toRestore = null; return@let }
        AlertDialog(
            onDismissRequest = { toRestore = null; pending = null },
            title = { Text("Restore ${r.name}?") },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Text("Upload $fileName (${data.size / 1024} KB) to ${r.name}.")
                    Spacer(Modifier.height(8.dp))
                    Text("The device restarts with that whole configuration: Wi-Fi, MQTT, topic, name, GPIOs, rules, SetOptions.")
                    Spacer(Modifier.height(8.dp))
                    Text("A file from another device copies its name, topic and network settings too: use one made from this device.", color = Orange)
                }
            },
            confirmButton = {
                TextButton(onClick = { toRestore = null; runRestore(r, data, null) }) { Text("Restore and restart") }
            },
            dismissButton = { TextButton(onClick = { toRestore = null; pending = null }) { Text("Cancel") } },
        )
    }
    error?.let { msg ->
        AlertDialog(
            onDismissRequest = { error = null },
            title = { Text("Backup / restore") },
            text = { Text(msg) },
            confirmButton = { TextButton(onClick = { error = null }) { Text("OK") } },
        )
    }
    askPassword?.let { (wrong, retry) ->
        var pw by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { askPassword = null },
            title = { Text("Web password") },
            text = {
                Column {
                    Text(if (wrong) "Wrong web password: try again." else "The device's web interface has a password.")
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = pw, onValueChange = { pw = it }, singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text("Used for this request only, never stored.", style = MaterialTheme.typography.bodySmall)
                }
            },
            confirmButton = { TextButton(onClick = { askPassword = null; retry(pw) }, enabled = pw.isNotEmpty()) { Text("OK") } },
            dismissButton = { TextButton(onClick = { askPassword = null }) { Text("Cancel") } },
        )
    }
    return files
}

/**
 * The device's two Wi-Fi networks. Only the one it is NOT using can be changed: changing the
 * one in use could leave it unable to reconnect (switch it to the other one first).
 */
@Composable
fun WifiDialog(
    d: Device, readOnly: Boolean, onDismiss: () -> Unit, onReadOnly: () -> Unit,
    onSave: (Int, String, String) -> Unit, onRestart: () -> Unit,
) {
    val active = d.ap?.toInt()
    val slot = when (active) { 1 -> 2; 2 -> 1; else -> null }
    val current = if (slot == 1) d.ssid1 else d.ssid2
    var ssid by remember(d.topic, slot) { mutableStateOf(current ?: "") }
    var password by remember(d.topic, slot) { mutableStateOf("") }
    var confirm by remember { mutableStateOf(false) }
    val ssidOk = ssid.trim().let { it.isNotEmpty() && it.length <= 32 && ';' !in it && it != "0" && it != "1" }
    val pwOk = password.length in 8..63 && ';' !in password

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("${d.name}: Wi-Fi networks") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                for (i in 1..2) {
                    val name = if (i == 1) d.ssid1 else d.ssid2
                    Text(
                        "SSId$i: ${name ?: "not set"}" + if (active == i) "  · in use" else "",
                        color = if (active == i) Green else MaterialTheme.colorScheme.onSurface,
                        fontWeight = if (active == i) FontWeight.SemiBold else FontWeight.Normal,
                    )
                }
                Spacer(Modifier.height(8.dp))
                Text(
                    "Only the network the device is not using can be changed: changing the one in use could leave it " +
                        "unable to reconnect. To change that one, switch the device to the other network first.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (slot == null) {
                    Spacer(Modifier.height(8.dp))
                    Text("Which network the device is using is not known yet: refresh and try again.", color = Orange)
                } else if (!confirm) {
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = ssid, onValueChange = { ssid = it.take(32) }, singleLine = true,
                        label = { Text("SSId$slot name") }, modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = password, onValueChange = { password = it.take(63) }, singleLine = true,
                        label = { Text("Password (8–63 characters)") },
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        isError = password.isNotEmpty() && !pwOk,
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "SSId$slot becomes \"${ssid.trim()}\". The device restarts and stays on SSId$active (the network it " +
                            "uses now); it moves to the new one only if that one is lost, or when you switch it.",
                    )
                    Spacer(Modifier.height(6.dp))
                    Text("The password goes to the device only: it is neither stored nor shown here.", style = MaterialTheme.typography.bodySmall)
                }
                if (!confirm) {
                    Spacer(Modifier.height(10.dp))
                    androidx.compose.material3.OutlinedButton(
                        onClick = { if (readOnly) onReadOnly() else onRestart() }, enabled = d.online == true,
                    ) { Text("Restart") }
                }
            }
        },
        confirmButton = {
            if (slot != null) {
                if (!confirm) {
                    TextButton(
                        onClick = { if (readOnly) onReadOnly() else confirm = true },
                        enabled = ssidOk && pwOk && d.online == true,
                    ) { Text("Save…") }
                } else {
                    TextButton(onClick = { onSave(slot, ssid.trim(), password); onDismiss() }) { Text("Save and restart") }
                }
            }
        },
        dismissButton = { TextButton(onClick = { if (confirm) confirm = false else onDismiss() }) { Text(if (confirm) "Back" else "Close") } },
    )
}
