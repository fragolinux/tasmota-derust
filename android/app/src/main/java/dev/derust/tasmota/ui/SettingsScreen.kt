package dev.derust.tasmota.ui

import androidx.compose.material3.AlertDialog
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.text.font.FontWeight
import dev.derust.tasmota.core.Changelog
import dev.derust.tasmota.core.changelog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.derust.tasmota.BrokerSettings
import dev.derust.tasmota.ThemeMode
import dev.derust.tasmota.core.aboutText
import dev.derust.tasmota.core.siteUrl
import androidx.compose.foundation.clickable
import dev.derust.tasmota.ui.theme.Orange
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    current: BrokerSettings,
    canClose: Boolean,
    onSave: (BrokerSettings) -> Unit,
    onClose: () -> Unit,
    appUpdate: String? = null,
    appReleases: String = "",
    theme: ThemeMode = ThemeMode.System,
    onTheme: (ThemeMode) -> Unit = {},
) {
    var host by rememberSaveable { mutableStateOf(current.host) }
    var port by rememberSaveable { mutableStateOf(current.port.toString()) }
    var user by rememberSaveable { mutableStateOf(current.username) }
    var pass by rememberSaveable { mutableStateOf(current.password) }
    var showPass by rememberSaveable { mutableStateOf(false) }

    val portNum = port.toIntOrNull()?.takeIf { it in 1..65535 }
    val valid = host.isNotBlank() && portNum != null

    if (canClose) BackHandler(onBack = onClose)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    if (canClose) IconButton(onClick = onClose) { Icon(Icons.Back, "Back") }
                },
            )
        },
    ) { pad ->
        Column(
            Modifier.fillMaxSize().padding(pad).imePadding().verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("MQTT broker", style = MaterialTheme.typography.titleMedium)
            OutlinedTextField(
                host, { host = it }, label = { Text("Host") }, singleLine = true,
                placeholder = { Text("192.168.1.10") },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                port, { port = it.filter(Char::isDigit).take(5) }, label = { Text("Port") }, singleLine = true,
                isError = portNum == null,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                user, { user = it }, label = { Text("Username") }, singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                pass, { pass = it }, label = { Text("Password") }, singleLine = true,
                visualTransformation = if (showPass) VisualTransformation.None else PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                trailingIcon = { TextButton(onClick = { showPass = !showPass }) { Text(if (showPass) "Hide" else "Show") } },
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(Modifier.padding(top = 4.dp))
            Button(
                onClick = { onSave(BrokerSettings(host.trim(), portNum ?: 1883, user, pass)) },
                enabled = valid,
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Save and connect") }
            Text(
                "Settings are stored on this phone only and are not included in backups. " +
                    "The app always starts in read-only mode: use the lock in the top bar to allow commands.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text("Appearance", style = MaterialTheme.typography.titleMedium)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                ThemeMode.entries.forEachIndexed { i, t ->
                    SegmentedButton(
                        selected = theme == t,
                        onClick = { onTheme(t) },
                        shape = SegmentedButtonDefaults.itemShape(i, ThemeMode.entries.size),
                    ) { Text(t.title) }
                }
            }

            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            val context = LocalContext.current
            val version = remember {
                runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()
            }
            Text("About Tasmota Derust ${version ?: ""}", style = MaterialTheme.typography.titleMedium)
            Text(aboutText(), style = MaterialTheme.typography.bodyMedium)
            var showNotes by remember { mutableStateOf<String?>(null) }
            val uri = LocalUriHandler.current
            TextButton(onClick = { showNotes = version }, enabled = version != null) {
                Text("What changed in this version")
            }
            if (appUpdate != null) {
                TextButton(onClick = { showNotes = appUpdate }) { Text("Version $appUpdate is available: what changed") }
            }
            TextButton(onClick = { uri.openUri(siteUrl()) }) { Text("Website and downloads") }
            showNotes?.let { v -> ChangelogDialog(v, onClose = { showNotes = null }) }
        }
    }
}

/**
 * About: first the latest release (download + what changed, loaded here, no browser needed),
 * then its release page, what the app is, and the project website.
 */
@Composable
fun AboutDialog(version: String?, latest: String?, update: String?, onClose: () -> Unit) {
    val uri = LocalUriHandler.current
    // `update` is set only when the latest release is newer than this app. Newer than the
    // latest release (being released, or a development build): its notes are not out yet.
    val ahead = update == null && latest != null && version != null && latest != version
    val target = if (ahead) null else update ?: version
    var notes by remember(target) { mutableStateOf<Changelog?>(null) }
    LaunchedEffect(target) { if (target != null) notes = withContext(Dispatchers.IO) { changelog(target) } }
    val newer = update != null
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Tasmota Derust ${version ?: ""}") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(
                    when {
                        newer -> "Version $update is available"
                        ahead -> "This version is newer than the latest release ($latest)"
                        latest != null -> "You have the latest version"
                        else -> "This version"
                    },
                    fontWeight = FontWeight.Bold,
                    color = if (newer) Orange else MaterialTheme.colorScheme.onSurface,
                )
                val n = notes
                n?.apkUrl?.let { apk ->
                    Button(onClick = { uri.openUri(apk) }, modifier = Modifier.padding(vertical = 6.dp)) {
                        Text("Download ${n.apkName ?: "the APK"}")
                    }
                }
                when {
                    target == null -> {}
                    n == null -> Text("Loading what changed…", style = MaterialTheme.typography.bodySmall)
                    n.error != null -> Text(n.error!!, style = MaterialTheme.typography.bodySmall)
                    else -> n.sections.forEach { sec ->
                        Text(sec.title, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 4.dp))
                        sec.items.forEach { Text("• $it", style = MaterialTheme.typography.bodyMedium) }
                    }
                }
                if (n != null) {
                    LinkText("Release page") { uri.openUri(n.url) }
                }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                Text(aboutText(), style = MaterialTheme.typography.bodyMedium)
                LinkText("Website: screenshots, guides, all downloads") { uri.openUri(siteUrl()) }
                Text(
                    "Theme (light / dark) and the MQTT broker are in Settings.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
    )
}

/** A link inside dialog text: aligned with the text around it, never clipped. */
@Composable
private fun LinkText(text: String, onClick: () -> Unit) {
    Text(
        text,
        color = MaterialTheme.colorScheme.primary,
        fontWeight = FontWeight.SemiBold,
        style = MaterialTheme.typography.bodyLarge,
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 10.dp),
    )
}

/** Release notes of a Tasmota Derust version, loaded from GitHub, with its APK to download. */
@Composable
fun ChangelogDialog(version: String, onClose: () -> Unit) {
    var notes by remember(version) { mutableStateOf<Changelog?>(null) }
    LaunchedEffect(version) { notes = withContext(Dispatchers.IO) { changelog(version) } }
    val uri = LocalUriHandler.current
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Tasmota Derust $version") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                val n = notes
                when {
                    n == null -> Text("Loading the release notes…")
                    n.error != null -> Text(n.error!!)
                    n.sections.isEmpty() -> Text("No list of changes in these release notes.")
                    else -> n.sections.forEach { sec ->
                        Text(sec.title, fontWeight = FontWeight.Bold)
                        sec.items.forEach { Text("• $it") }
                        Spacer(Modifier.padding(top = 6.dp))
                    }
                }
                val apk = n?.apkUrl
                if (apk != null) {
                    Button(onClick = { uri.openUri(apk) }, modifier = Modifier.padding(top = 8.dp)) {
                        Text("Download ${n.apkName ?: "the APK"}")
                    }
                } else if (n != null && n.error == null) {
                    Text("No APK in this release.", style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
        dismissButton = {
            notes?.let { n -> TextButton(onClick = { uri.openUri(n.url) }) { Text("Release page") } }
        },
    )
}
