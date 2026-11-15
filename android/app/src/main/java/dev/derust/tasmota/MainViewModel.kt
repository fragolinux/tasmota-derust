package dev.derust.tasmota

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.derust.tasmota.core.Broker
import dev.derust.tasmota.core.CommandException
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
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

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
