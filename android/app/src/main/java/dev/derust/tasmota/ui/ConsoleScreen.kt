package dev.derust.tasmota.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.derust.tasmota.MainViewModel.ConsoleState
import dev.derust.tasmota.core.Favourite
import dev.derust.tasmota.core.consoleRisk
import dev.derust.tasmota.core.isTelemetry
import dev.derust.tasmota.ui.theme.Orange
import dev.derust.tasmota.ui.theme.Red

/**
 * One device's console, full screen: its MQTT messages, a command line with the device's
 * history (arrows) and the favourite commands (a tap puts one in the command line).
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun ConsoleScreen(
    name: String,
    online: Boolean,
    readOnly: Boolean,
    state: ConsoleState,
    favourites: List<Favourite>,
    onClose: () -> Unit,
    onSend: (String, Boolean, () -> Unit) -> Unit,
    onSaveFavourites: (List<Favourite>) -> Unit,
) {
    var input by rememberSaveable { mutableStateOf("") }
    // Where the arrows are in the history (null: the line being typed). Same rule as the core.
    var pos by remember { mutableStateOf<Int?>(null) }
    var hideTele by rememberSaveable { mutableStateOf(false) }
    var editing by remember { mutableStateOf(false) }
    val listState = rememberLazyListState()
    val shown = if (hideTele) state.lines.filter { it.sent || !isTelemetry(it.topic) } else state.lines

    // Follow new messages while the list is at the bottom.
    // The first messages always open at the end.
    val last = shown.lastOrNull()?.seq
    var filled by remember { mutableStateOf(false) }
    // After a send the answer is followed, until the user scrolls the list.
    var stick by remember { mutableStateOf(false) }
    LaunchedEffect(listState.isScrollInProgress) {
        if (listState.isScrollInProgress) stick = false
    }
    LaunchedEffect(last) {
        if (shown.isEmpty()) return@LaunchedEffect
        val info = listState.layoutInfo
        val lastVisible = info.visibleItemsInfo.lastOrNull()?.index ?: -1
        if (!filled || stick || lastVisible >= info.totalItemsCount - 3) {
            listState.scrollToItem(shown.size - 1)
            filled = true
        }
    }

    fun browse(older: Boolean) {
        val n = state.history.size
        if (n == 0) return
        pos = when {
            older -> pos?.let { maxOf(0, it - 1) } ?: (n - 1)
            pos == null || pos!! + 1 >= n -> null
            else -> pos!! + 1
        }
        input = pos?.let { state.history[it] } ?: ""
    }

    // A line that can cut the device off (Wi-Fi, MQTT, reset) asks first.
    var risky by remember { mutableStateOf<Pair<String, String>?>(null) }
    fun send(confirmed: Boolean = false) {
        val line = input.trim()
        if (line.isEmpty()) return
        val why = if (confirmed || readOnly) null else consoleRisk(line)
        if (why != null) {
            risky = line to why
            return
        }
        onSend(line, confirmed) {
            input = ""
            pos = null
            stick = true
        }
    }

    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("$name — Console", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = { IconButton(onClick = onClose) { Icon(Icons.Back, "Back") } },
                )
            },
            bottomBar = {
                Column(Modifier.navigationBarsPadding().imePadding()) {
                    HorizontalDivider()
                    Row(
                        Modifier.fillMaxWidth().padding(start = 12.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        OutlinedTextField(
                            value = input,
                            onValueChange = { input = it; pos = null },
                            placeholder = { Text("Command, e.g. Status 0") },
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                            keyboardOptions = KeyboardOptions(
                                capitalization = KeyboardCapitalization.None,
                                autoCorrectEnabled = false,
                                imeAction = ImeAction.Send,
                            ),
                            keyboardActions = KeyboardActions(onSend = { send() }),
                            modifier = Modifier.weight(1f),
                        )
                        IconButton(onClick = { browse(true) }, enabled = state.history.isNotEmpty() && pos != 0) {
                            Icon(Icons.Up, "Older command")
                        }
                        IconButton(onClick = { browse(false) }, enabled = pos != null) {
                            Icon(Icons.Down, "Newer command")
                        }
                    }
                }
            },
        ) { pad ->
            Column(Modifier.fillMaxSize().padding(pad)) {
                // On several lines: everything stays in view, Edit included.
                FlowRow(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    for (f in favourites) {
                        if (f.command.isBlank()) continue
                        AssistChip(
                            onClick = { input = f.command; pos = null },
                            label = { Text(f.name.ifBlank { f.command }, maxLines = 1) },
                        )
                    }
                    AssistChip(onClick = { editing = true }, label = { Text("Edit favourites") })
                    FilterChip(selected = hideTele, onClick = { hideTele = !hideTele }, label = { Text("Hide telemetry") })
                }
                when {
                    readOnly -> Note("Read-only is on: messages are shown, commands are not sent.", Orange)
                    !online -> Note("The device is not online: commands are not sent.", Red)
                }
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                ) {
                    if (shown.isEmpty()) item {
                        Text("No messages yet.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    items(shown, key = { it.seq.toLong() }) { l ->
                        val color = when {
                            l.sent -> Orange
                            l.retained -> MaterialTheme.colorScheme.onSurfaceVariant
                            else -> MaterialTheme.colorScheme.onSurface
                        }
                        Text(
                            "${l.time} ${if (l.sent) ">" else "<"} ${l.topic}${if (l.retained) " (retained)" else ""}  ${l.payload}",
                            color = color, fontFamily = FontFamily.Monospace, fontSize = 11.5.sp, lineHeight = 15.sp,
                            modifier = Modifier.padding(vertical = 2.dp),
                        )
                    }
                }
            }
        }
    }

    risky?.let { (line, why) ->
        AlertDialog(
            onDismissRequest = { risky = null },
            title = { Text("Send this to $name?") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(line, fontFamily = FontFamily.Monospace)
                    Text(why, color = Orange)
                }
            },
            confirmButton = { TextButton(onClick = { risky = null; send(confirmed = true) }) { Text("Send anyway") } },
            dismissButton = { TextButton(onClick = { risky = null }) { Text("Cancel") } },
        )
    }

    if (editing) FavouritesDialog(favourites, onDismiss = { editing = false }) {
        onSaveFavourites(it)
        editing = false
    }
}

@Composable
private fun Note(text: String, color: androidx.compose.ui.graphics.Color) {
    Text(text, color = color, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
}

@Composable
private fun FavouritesDialog(current: List<Favourite>, onDismiss: () -> Unit, onSave: (List<Favourite>) -> Unit) {
    val names = remember { mutableStateListOf(*current.map { it.name }.toTypedArray()) }
    val cmds = remember { mutableStateListOf(*current.map { it.command }.toTypedArray()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Favourite commands") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "A tap puts the command in the command line, ready to edit and send. Use Backlog for several " +
                        "commands; replace parts like <password> before sending. Saved as plain text on this phone.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                for (i in names.indices) {
                    OutlinedTextField(
                        value = names[i], onValueChange = { names[i] = it },
                        label = { Text("Name ${i + 1}") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = cmds[i], onValueChange = { cmds[i] = it },
                        label = { Text("Command ${i + 1}") },
                        textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onSave(names.indices.map { Favourite(names[it], cmds[it]) }) }) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
