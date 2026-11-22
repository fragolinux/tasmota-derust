package dev.derust.tasmota

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.derust.tasmota.core.BackupException
import dev.derust.tasmota.core.BackupFile
import dev.derust.tasmota.core.Broker
import dev.derust.tasmota.core.CommandException
import dev.derust.tasmota.core.ConsoleLine
import dev.derust.tasmota.core.Favourite
import dev.derust.tasmota.core.MqttDefaults
import dev.derust.tasmota.core.SettingsException
import dev.derust.tasmota.core.Options
import dev.derust.tasmota.core.Session
import dev.derust.tasmota.core.Snapshot
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Owns the Rust [Session] (MQTT connection + device store) and polls it once a second
 * while the screen is visible.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModel(app: Application) : AndroidViewModel(app) {
    private val prefs = Prefs(app)

    private val _settings = MutableStateFlow(prefs.broker())
    val settings: StateFlow<BrokerSettings> = _settings.asStateFlow()

    private val session = MutableStateFlow<Session?>(null)

    val snapshot: StateFlow<Snapshot?> = session
        .flatMapLatest<Session?, Snapshot?> { s ->
            if (s == null) flowOf(null) else flow {
                while (true) {
                    emit(runCatching { s.snapshot() }.onFailure { android.util.Log.w("derust", "snapshot failed", it) }.getOrNull())
                    delay(1000)
                }
            }
        }
        .flowOn(Dispatchers.Default)
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** Device whose SetOptions screen is open. */
    private val _optionsTopic = MutableStateFlow<String?>(null)
    val optionsTopic: StateFlow<String?> = _optionsTopic.asStateFlow()

    /** SetOptions of [optionsTopic], polled while the screen is open. */
    val options: StateFlow<Options?> = combine(session, _optionsTopic) { s, t -> s to t }
        .flatMapLatest<Pair<Session?, String?>, Options?> { (s, t) ->
            if (s == null || t == null) flowOf(null) else flow {
                while (true) {
                    emit(runCatching { s.options(t) }.getOrNull())
                    delay(1000)
                }
            }
        }
        .flowOn(Dispatchers.Default)
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    fun openOptions(topic: String?) {
        _optionsTopic.value = topic
    }

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 4)
    val messages = _messages.asSharedFlow()

    /** Fires when a command was attempted while read-only is on (nothing was sent). */
    private val _readOnlyHits = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    val readOnlyHits = _readOnlyHits.asSharedFlow()

    /**
     * Read-only: only read queries reach the devices; relays, Wi-Fi switching and upgrades
     * are refused. On at every launch, switched from the lock in the top bar.
     */
    var readOnly by mutableStateOf(true)
        private set

    // Screen state, remembered between launches.
    // Opens on the dashboard.
    var view by mutableStateOf(View.Dashboard)
        private set

    /** Ticking devices (group upgrade, Enable MQTT). */
    var selecting by mutableStateOf(false)
    var selected by mutableStateOf(setOf<String>())
        private set

    fun select(topic: String, on: Boolean) {
        selected = if (on) selected + topic else selected - topic
    }

    fun selectAll(topics: List<String>) {
        selected = selected + topics
    }

    fun stopSelecting() {
        selecting = false
        selected = emptySet()
    }

    private fun failures(list: List<dev.derust.tasmota.core.Failure>, ok: String) {
        _messages.tryEmit(if (list.isEmpty()) ok else list.joinToString(" · ") { "${it.topic}: ${it.error}" })
    }

    /** Upgrades several devices at once, each on its own (path steps follow by themselves). */
    fun upgradeMany(topics: List<String>) = command(null) { s ->
        failures(s.upgradeMany(topics), "Upgrade started on ${topics.size} device(s)")
    }

    /** Connects HTTP-only devices to the broker; `password` null: this app's own. */
    fun enableMqtt(topics: List<String>, host: String, port: Int, user: String, password: String?) = command(null) { s ->
        failures(
            s.enableMqtt(topics, host, port.toUShort(), user, password),
            "MQTT settings sent: the devices restart and show up over MQTT",
        )
        stopSelecting()
    }

    /** Searches the network over HTTP (read-only): progress in the snapshot. */
    fun search(range: String, password: String) = run(null) { s ->
        try {
            s.search(range, password.ifEmpty { null })
        } catch (e: SettingsException) {
            _messages.tryEmit(e.message ?: "Search refused")
        }
    }

    fun setHttpPassword(topic: String, password: String) = run(null) { it.setHttpPassword(topic, password) }

    fun mqttDefaults(): MqttDefaults? = runCatching { session.value?.mqttDefaults() }.getOrNull()
    var theme by mutableStateOf(prefs.theme)
        private set

    fun selectTheme(t: ThemeMode) {
        theme = t
        prefs.theme = t
    }

    // Offline devices hidden at every start.
    var showOffline by mutableStateOf(false)
        private set
    var sort by mutableStateOf(prefs.sort)
        private set
    var search by mutableStateOf("")

    init {
        // Console history and favourites live in the app's files directory.
        dev.derust.tasmota.core.setDataDir(app.filesDir.path)
        connect()
    }

    /** What the console screen shows: the device's recent messages and its command history. */
    data class ConsoleState(val lines: List<ConsoleLine> = emptyList(), val history: List<String> = emptyList())

    /** Device whose console is open. */
    private val _consoleTopic = MutableStateFlow<String?>(null)
    val consoleTopic: StateFlow<String?> = _consoleTopic.asStateFlow()
    private val _console = MutableStateFlow(ConsoleState())
    val console: StateFlow<ConsoleState> = _console.asStateFlow()
    private var consoleJob: Job? = null

    /** Favourite console commands (5 slots, shared by all devices). */
    var favourites by mutableStateOf(dev.derust.tasmota.core.favourites())
        private set

    fun saveFavourites(list: List<Favourite>) {
        dev.derust.tasmota.core.setFavourites(list)
        favourites = dev.derust.tasmota.core.favourites()
    }

    /** Opens (or, with null, closes) a device's console; its messages are polled while open. */
    fun openConsole(topic: String?) {
        consoleJob?.cancel()
        _consoleTopic.value = topic
        _console.value = ConsoleState()
        if (topic == null) return
        consoleJob = viewModelScope.launch(Dispatchers.Default) {
            var after = 0uL
            var from: Session? = null
            while (true) {
                val s = session.value
                if (s !== from) {
                    // A new connection numbers its messages from the start again.
                    from = s
                    after = 0uL
                    _console.value = ConsoleState()
                }
                val v = s?.let { runCatching { it.console(topic, after) }.getOrNull() }
                if (v != null) {
                    v.lines.lastOrNull()?.let { after = it.seq }
                    val lines = if (v.lines.isEmpty()) _console.value.lines
                    else (_console.value.lines + v.lines).takeLast(CONSOLE_MAX)
                    _console.value = ConsoleState(lines, v.history)
                }
                delay(700)
            }
        }
    }

    /**
     * Sends a console line; [onSent] runs when the device got it (the screen clears the line).
     * Lines that can cut the device off need [confirmed] (the screen asks first).
     */
    fun consoleSend(topic: String, line: String, confirmed: Boolean, onSent: () -> Unit) {
        if (readOnly) return readOnlyHit()
        val s = session.value ?: return
        viewModelScope.launch(Dispatchers.Default) {
            try {
                s.consoleSend(topic, line, confirmed)
                withContext(Dispatchers.Main) { onSent() }
            } catch (e: CommandException.ReadOnly) {
                readOnlyHit()
            } catch (e: CommandException) {
                _messages.tryEmit(e.message ?: "Command refused")
            } catch (e: IllegalStateException) {
                // Reconnecting: nothing was sent.
            }
        }
    }

    /** A short hint that fades out by itself (Android toast), not in the way. */
    private fun hint(text: String) {
        android.widget.Toast.makeText(getApplication(), text, android.widget.Toast.LENGTH_SHORT).show()
    }

    fun selectView(v: View) {
        view = v
        if (v == View.Dashboard && showOffline) hint(DASHBOARD_ONLINE)
        prefs.view = v
        if (sort !in v.sorts) selectSort(SortKey.Name)
    }

    fun changeReadOnly(on: Boolean) {
        readOnly = on
        session.value?.setReadOnly(on)
        _messages.tryEmit(if (on) "Read-only on: commands are blocked" else "Read-only off: commands are sent to devices")
    }

    fun toggleOffline() {
        showOffline = !showOffline
        if (showOffline && view == View.Dashboard) hint(DASHBOARD_ONLINE)
    }

    fun selectSort(s: SortKey) {
        sort = s
        prefs.sort = s
    }

    fun saveSettings(b: BrokerSettings) {
        prefs.saveBroker(b)
        _settings.value = prefs.broker()
        connect()
    }

    /** (Re)connects with the saved settings. The old session is closed first. */
    private fun connect() {
        val old = session.value
        session.value = null
        val b = _settings.value
        viewModelScope.launch(Dispatchers.Default) {
            old?.close()
            if (b.host.isBlank()) return@launch
            session.value = Session(
                Broker(b.host.trim(), b.port.toUShort(), b.username, b.password),
                readOnly,
                b.topicPatterns,
                b.topicDevices,
            )
        }
    }

    /** Read queries only (Status 0, Module, SSId): fine in read-only mode. */
    fun refresh() = run("Asking devices for status…") { it.refresh() }

    fun togglePower(topic: String, relay: Int) = command(null) { it.togglePower(topic, relay.toUInt()) }

    fun switchAp(topic: String, ap: Int) = command("Switching Wi-Fi…") { it.switchAp(topic, ap.toUByte()) }

    fun upgrade(topic: String) = command("Upgrade started") { it.upgrade(topic) }
    fun restart(topic: String) = command("Restarting…") { it.restart(topic) }
    fun forget(topic: String) = command("Device removed") { it.forget(topic) }
    fun forgetOffline() = command(null) { s ->
        val n = s.forgetOffline().toInt()
        _messages.tryEmit(if (n == 1) "1 device removed" else "$n devices removed")
    }

    fun setDimmer(topic: String, v: Long) = command(null) { it.setDimmer(topic, v) }
    fun setHsb(topic: String, h: Long, s: Long, b: Long) = command(null) { it.setHsb(topic, h, s, b) }
    fun setCt(topic: String, v: Long) = command(null) { it.setCt(topic, v) }
    fun setWhite(topic: String, v: Long) = command(null) { it.setWhite(topic, v) }

    fun setChannel(topic: String, development: Boolean) =
        command(if (development) "OtaUrl: development builds" else "OtaUrl: releases") { it.setChannel(topic, development) }

    /** `SetOption<n> <value>`; the options are read back afterwards. */
    fun setOption(topic: String, n: Int, value: Long) =
        command("SetOption$n $value sent") { it.setOption(topic, n.toUShort(), value.toUInt()) }

    /** Sets the Wi-Fi network the device is NOT using; the device restarts once. */
    fun setWifi(topic: String, slot: Int, ssid: String, password: String) =
        command("SSId$slot sent: the device restarts") { it.setWifi(topic, slot.toUByte(), ssid, password) }

    /** Outcome of a backup or restore, for the screen. */
    sealed interface FileResult<out T> {
        data class Ok<T>(val value: T) : FileResult<T>
        /** The device's web interface has a password (or the one given was wrong). */
        data class NeedsPassword(val wrong: Boolean) : FileResult<Nothing>
        data class Error(val message: String) : FileResult<Nothing>
    }

    private suspend fun <T> files(block: (Session) -> T): FileResult<T> = withContext(Dispatchers.IO) {
        val s = session.value ?: return@withContext FileResult.Error("Not connected")
        try {
            FileResult.Ok(block(s))
        } catch (e: BackupException.NeedsPassword) {
            FileResult.NeedsPassword(false)
        } catch (e: BackupException.WrongPassword) {
            FileResult.NeedsPassword(true)
        } catch (e: BackupException.Failed) {
            FileResult.Error(e.reason)
        } catch (e: BackupException.NoAddress) {
            FileResult.Error("The device's IP address is not known yet.")
        } catch (e: BackupException) {
            FileResult.Error("Failed")
        } catch (e: IllegalStateException) {
            FileResult.Error("Not connected")
        }
    }

    /** The device's whole configuration (.dmp) from its web server. Allowed in read-only. */
    suspend fun backup(topic: String, password: String?): FileResult<BackupFile> =
        files { it.backup(topic, password) }

    /** Uploads a configuration file; the device restarts with it. Never in read-only. */
    suspend fun restore(topic: String, password: String?, data: ByteArray): FileResult<Unit> {
        if (readOnly) {
            readOnlyHit()
            return FileResult.Error("Read-only mode is on: nothing was sent.")
        }
        return files { it.restore(topic, password, data) }
    }

    /** Tells the user that read-only is on (used before asking for a confirmation, too). */
    fun readOnlyHit() {
        _readOnlyHits.tryEmit(Unit)
    }

    /** A state-changing command: never sent in read-only mode (the Rust core refuses it too). */
    private fun command(okMessage: String?, block: (Session) -> Unit) {
        if (readOnly) return readOnlyHit()
        run(okMessage, block)
    }

    private fun run(okMessage: String?, block: (Session) -> Unit) {
        val s = session.value ?: return
        viewModelScope.launch(Dispatchers.Default) {
            try {
                block(s)
                okMessage?.let { _messages.tryEmit(it) }
            } catch (e: CommandException.ReadOnly) {
                readOnlyHit()
            } catch (e: CommandException) {
                _messages.tryEmit(e.message ?: "Command refused")
            } catch (e: IllegalStateException) {
                // The session was closed while reconnecting; nothing was sent.
            }
        }
    }

    private companion object {
        const val CONSOLE_MAX = 300
        const val DASHBOARD_ONLINE = "The dashboard shows online devices only"
    }

    override fun onCleared() {
        session.value?.close()
    }
}
