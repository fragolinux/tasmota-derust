package dev.derust.tasmota

import android.content.Context
import androidx.core.content.edit

/** Broker settings. Stored in the app's private preferences, never backed up (allowBackup=false). */
data class BrokerSettings(
    val host: String = "",
    val port: Int = 1883,
    val username: String = "",
    val password: String = "",
)

class Prefs(context: Context) {
    private val p = context.getSharedPreferences("derust", Context.MODE_PRIVATE)

    fun broker() = BrokerSettings(
        host = p.getString("host", "") ?: "",
        port = p.getInt("port", 1883),
        username = p.getString("username", "") ?: "",
        password = p.getString("password", "") ?: "",
    )

    fun saveBroker(b: BrokerSettings) = p.edit {
        putString("host", b.host.trim())
        putInt("port", b.port)
        putString("username", b.username)
        putString("password", b.password)
        remove("readOnly") // was a broker setting; read-only now starts on at every launch
    }

    var view: View
        get() = View.entries.firstOrNull { it.name == p.getString("view", null) } ?: View.Home
        set(v) = p.edit { putString("view", v.name) }

    var showOffline: Boolean
        get() = p.getBoolean("showOffline", false)
        set(v) = p.edit { putBoolean("showOffline", v) }

    var sort: SortKey
        get() = SortKey.entries.firstOrNull { it.name == p.getString("sort", null) } ?: SortKey.Name
        set(v) = p.edit { putString("sort", v.name) }
}
