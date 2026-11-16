package dev.derust.tasmota.ui

import androidx.compose.foundation.background
import androidx.compose.ui.platform.LocalContext
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.LocalContentColor
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.derust.tasmota.MainViewModel
import dev.derust.tasmota.View
import dev.derust.tasmota.isOffline
import dev.derust.tasmota.matches
import dev.derust.tasmota.sortedFor
import dev.derust.tasmota.ui.theme.Green
import dev.derust.tasmota.ui.theme.OnStatus
import dev.derust.tasmota.ui.theme.Orange
import dev.derust.tasmota.ui.theme.Red
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun App(vm: MainViewModel) {
    val settings by vm.settings.collectAsStateWithLifecycle()
    val snap by vm.snapshot.collectAsStateWithLifecycle()
    var showSettings by rememberSaveable { mutableStateOf(false) }
    if (showSettings || settings.host.isBlank()) {
        SettingsScreen(
            current = settings,
            canClose = settings.host.isNotBlank(),
            onSave = { vm.saveSettings(it); showSettings = false },
            onClose = { showSettings = false },
            appUpdate = snap?.appUpdate,
            appReleases = snap?.appReleases ?: "",
            theme = vm.theme,
            onTheme = vm::selectTheme,
        )
    } else {
        MainScreen(vm, onSettings = { showSettings = true })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MainScreen(vm: MainViewModel, onSettings: () -> Unit) {
    val snap by vm.snapshot.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    var openTopic by rememberSaveable { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val context = LocalContext.current
    val appVersion = remember {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()
    }
    // Release notes shown for this version (the running one, or the available update).
    var showNotes by remember { mutableStateOf<String?>(null) }
    showNotes?.let { v -> ChangelogDialog(v, onClose = { showNotes = null }) }
    var showAbout by remember { mutableStateOf(false) }
    if (showAbout) {
        AboutDialog(appVersion, onNotes = { showAbout = false; showNotes = appVersion }, onClose = { showAbout = false })
    }

    val files = rememberDeviceFiles(vm)
    var wifiTopic by rememberSaveable { mutableStateOf<String?>(null) }

    // null: hidden; true: a command was blocked; false: the lock was tapped.
    var askUnlock by remember { mutableStateOf<Boolean?>(null) }

    LaunchedEffect(Unit) { vm.messages.collect { snackbar.showSnackbar(it) } }
    LaunchedEffect(Unit) { vm.readOnlyHits.collect { askUnlock = true } }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("Tasmota Derust", fontWeight = FontWeight.SemiBold, fontSize = 20.sp, maxLines = 1)
                            Spacer(Modifier.width(6.dp))
                            // A tappable pill, so it reads as a button: opens About (what the
                            // app is, what changed, theme).
                            Row(
                                Modifier.clip(RoundedCornerShape(50))
                                    .background(MaterialTheme.colorScheme.secondaryContainer)
                                    .clickable { showAbout = true }
                                    .padding(start = 5.dp, end = 8.dp, top = 2.dp, bottom = 2.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Icon(
                                    Icons.Info, "About Tasmota Derust", Modifier.size(14.dp),
                                    tint = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                                Spacer(Modifier.width(3.dp))
                                Text(
                                    appVersion?.let { "v$it" } ?: "About", fontSize = 12.sp, maxLines = 1,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                            }
                            snap?.appUpdate?.let { v ->
                                Text(
                                    "  Update $v", fontSize = 13.sp, maxLines = 1, color = Orange,
                                    fontWeight = FontWeight.SemiBold,
                                    modifier = Modifier.clickable { showNotes = v },
                                )
                            }
                        }
                        ConnectionLine(snap?.connected, snap?.status, vm.readOnly)
                    }
                },
                actions = {
                    IconButton(onClick = { if (vm.readOnly) askUnlock = false else vm.changeReadOnly(true) }) {
                        if (vm.readOnly) Icon(Icons.Lock, "Read-only on: tap to allow commands", tint = Orange)
                        else Icon(Icons.Unlock, "Commands allowed: tap for read-only", tint = Red)
                    }
                    IconButton(onClick = vm::refresh, enabled = snap?.connected == true) { Icon(Icons.Refresh, "Refresh") }
                    IconButton(onClick = onSettings) { Icon(Icons.Settings, "Settings") }
                },
            )
        },
        bottomBar = {
            NavigationBar {
                for (v in View.entries) {
                    NavigationBarItem(
                        selected = vm.view == v,
                        onClick = { vm.selectView(v) },
                        icon = { Icon(v.icon, null) },
                        // One line, smaller when the phone's font size needs it ("Firmware" wrapped).
                        label = {
                            BasicText(
                                v.title, maxLines = 1, softWrap = false,
                                style = LocalTextStyle.current.copy(color = LocalContentColor.current),
                                autoSize = TextAutoSize.StepBased(minFontSize = 8.sp, maxFontSize = LocalTextStyle.current.fontSize),
                            )
                        },
                    )
                }
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad)) {
            Toolbar(vm, offlineCount = snap?.devices?.count { it.isOffline } ?: 0)
            val s = snap
            val list = s?.devices.orEmpty()
                .filter { (vm.showOffline || !it.isOffline || it.upgrading) && it.matches(vm.search) }
                .sortedFor(vm.sort)
            PullToRefreshBox(
                isRefreshing = refreshing,
                onRefresh = {
                    vm.refresh()
                    scope.launch { refreshing = true; delay(800); refreshing = false }
                },
                modifier = Modifier.fillMaxSize(),
            ) {
                when {
                    s == null -> Empty("Connecting…")
                    list.isEmpty() -> Empty(
                        when {
                            !s.connected -> "Not connected to the MQTT broker.\n${s.status}"
                            s.devices.isEmpty() -> "Waiting for Tasmota devices…"
                            else -> "No devices match."
                        },
                    )
                    else -> DeviceList(
                        snapshot = s, devices = list, view = vm.view,
                        contentPadding = PaddingValues(bottom = 16.dp),
                        onOpen = { openTopic = it.topic },
                        onToggle = { d, n -> vm.togglePower(d.topic, n) },
                    )
                }
            }
        }
    }

    val s = snap
    val open = s?.devices?.firstOrNull { it.topic == openTopic }
    if (s != null && open != null) {
        DeviceSheet(
            d = open, s = s,
            onDismiss = { openTopic = null },
            onToggle = { n -> vm.togglePower(open.topic, n) },
            onSwitchAp = { n -> vm.switchAp(open.topic, n) },
            onUpgrade = { vm.upgrade(open.topic) },
            onSetChannel = { dev -> vm.setChannel(open.topic, dev) },
            onHsb = { h, sat, b -> vm.setHsb(open.topic, h, sat, b) },
            onDimmer = { vm.setDimmer(open.topic, it) },
            onCt = { vm.setCt(open.topic, it) },
            onWhite = { vm.setWhite(open.topic, it) },
            onOptions = { vm.openOptions(open.topic) },
            onWifi = { wifiTopic = open.topic },
            onBackup = { files.backup(open) },
            onRestore = { files.restore(open) },
            readOnly = vm.readOnly,
            onReadOnly = vm::readOnlyHit,
        )
    }

    val optTopic by vm.optionsTopic.collectAsStateWithLifecycle()
    val opts by vm.options.collectAsStateWithLifecycle()
    val optDevice = s?.devices?.firstOrNull { it.topic == optTopic }
    if (optTopic != null && optDevice != null) {
        OptionsScreen(
            name = optDevice.name,
            online = optDevice.online == true,
            readOnly = vm.readOnly,
            opts = opts,
            onClose = { vm.openOptions(null) },
            onSet = { n, v -> vm.setOption(optDevice.topic, n, v) },
            onReadOnly = vm::readOnlyHit,
            onBackup = { files.backup(optDevice) },
        )
    }
    val wifiDevice = s?.devices?.firstOrNull { it.topic == wifiTopic }
    if (wifiTopic != null && wifiDevice != null) {
        WifiDialog(
            wifiDevice, readOnly = vm.readOnly,
            onDismiss = { wifiTopic = null },
            onReadOnly = vm::readOnlyHit,
            onSave = { slot, ssid, pw -> vm.setWifi(wifiDevice.topic, slot, ssid, pw) },
        )
    }

    askUnlock?.let { blocked ->
        AlertDialog(
            onDismissRequest = { askUnlock = null },
            title = { Text(if (blocked) "Read-only is on" else "Allow commands?") },
            text = {
                Text(
                    if (blocked) {
                        "Nothing was sent. While read-only is on, relays, Wi-Fi switching, SetOptions and upgrades are blocked."
                    } else {
                        "Relays, Wi-Fi switching, SetOptions and upgrades will be sent to the devices."
                    } + " Read-only turns back on every time the app starts.",
                )
            },
            confirmButton = {
                TextButton(onClick = { vm.changeReadOnly(false); askUnlock = null }) { Text("Allow commands") }
            },
            dismissButton = {
                TextButton(onClick = { askUnlock = null }) { Text("Keep read-only") }
            },
        )
    }
}

private val View.icon get() = when (this) {
    View.Home -> Icons.Home
    View.Health -> Icons.Health
    View.Firmware -> Icons.Firmware
    View.Wifi -> Icons.Wifi
    View.Mqtt -> Icons.Mqtt
}

@Composable
private fun ConnectionLine(connected: Boolean?, status: String?, readOnly: Boolean?) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier.size(8.dp).background(
                when (connected) { true -> Green; false -> Red; null -> MaterialTheme.colorScheme.outline },
                CircleShape,
            ),
        )
        Spacer(Modifier.width(6.dp))
        Text(
            when (connected) { true -> "Connected"; false -> status ?: "Disconnected"; null -> "Connecting…" },
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        if (readOnly == true) {
            Spacer(Modifier.width(8.dp))
            Text(
                "Read-only", fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = OnStatus,
                modifier = Modifier.background(Orange, RoundedCornerShape(4.dp)).padding(horizontal = 6.dp, vertical = 1.dp),
            )
        }
    }
}

@Composable
private fun Toolbar(vm: MainViewModel, offlineCount: Int) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // Compact search field (a Material TextField is 56 dp tall).
        Row(
            Modifier.weight(1f).height(40.dp)
                .background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(20.dp))
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Search, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.width(8.dp))
            Box(Modifier.weight(1f)) {
                if (vm.search.isEmpty()) Text("Search", color = MaterialTheme.colorScheme.onSurfaceVariant)
                BasicTextField(
                    vm.search, { vm.search = it }, singleLine = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            if (vm.search.isNotEmpty()) {
                IconButton(onClick = { vm.search = "" }, modifier = Modifier.size(28.dp)) {
                    Icon(Icons.Close, "Clear", Modifier.size(16.dp))
                }
            }
        }
        FilterChip(
            selected = vm.showOffline, onClick = vm::toggleOffline,
            label = { Text("Offline $offlineCount") },
        )
        Box {
            var open by remember { mutableStateOf(false) }
            IconButton(onClick = { open = true }) { Icon(Icons.Sort, "Sort by ${vm.sort.title}") }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                Text(
                    "Sort by", style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
                )
                for (k in vm.view.sorts) {
                    DropdownMenuItem(
                        text = { Text(k.title, fontWeight = if (k == vm.sort) FontWeight.Bold else null) },
                        onClick = { vm.selectSort(k); open = false },
                    )
                }
            }
        }
    }
}

@Composable
private fun Empty(text: String) {
    // Inside PullToRefreshBox: fill the space so the pull gesture works here too.
    Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.TopCenter) {
        Text(
            text, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 48.dp),
        )
    }
}
