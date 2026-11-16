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
import dev.derust.tasmota.core.Options
import dev.derust.tasmota.core.Session
import dev.derust.tasmota.core.Snapshot
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
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
                    emit(runCatching { s.snapshot() }.getOrNull())
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
    var view by mutableStateOf(prefs.view)
        private set
    var theme by mutableStateOf(prefs.theme)
        private set

    fun selectTheme(t: ThemeMode) {
        theme = t
        prefs.theme = t
    }

    var showOffline by mutableStateOf(prefs.showOffline)
        private set
    var sort by mutableStateOf(prefs.sort)
        private set
    var search by mutableStateOf("")

    init {
        connect()
    }

    fun selectView(v: View) {
        view = v
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
        prefs.showOffline = showOffline
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
            )
        }
    }

    /** Read queries only (Status 0, Module, SSId): fine in read-only mode. */
    fun refresh() = run("Asking devices for status…") { it.refresh() }

    fun togglePower(topic: String, relay: Int) = command(null) { it.togglePower(topic, relay.toUInt()) }

    fun switchAp(topic: String, ap: Int) = command("Switching Wi-Fi…") { it.switchAp(topic, ap.toUByte()) }

    fun upgrade(topic: String) = command("Upgrade started") { it.upgrade(topic) }

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

    override fun onCleared() {
        session.value?.close()
    }
}
