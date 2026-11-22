package dev.derust.tasmota

import dev.derust.tasmota.core.Device

/** The views, the same as the desktop app and the web page. */
enum class View(val title: String, val sorts: List<SortKey>) {
    Dashboard("Dashboard", listOf(SortKey.Name, SortKey.Module, SortKey.Uptime, SortKey.Rssi)),
    Home("Home", listOf(SortKey.Name, SortKey.Module, SortKey.Uptime, SortKey.Rssi)),
    Health("Health", listOf(SortKey.Name, SortKey.Uptime, SortKey.BootCount, SortKey.RestartReason, SortKey.LoadAvg, SortKey.Rssi)),
    Firmware("Firmware", listOf(SortKey.Name, SortKey.Version, SortKey.Core, SortKey.Module)),
    Wifi("Wifi", listOf(SortKey.Name, SortKey.Ip, SortKey.Ssid, SortKey.Ap, SortKey.Rssi, SortKey.Channel)),
    Mqtt("MQTT", listOf(SortKey.Name, SortKey.Topic)),
}

/** Text sorts A→Z, numbers biggest first. Missing values always go last. */
enum class SortKey(val title: String, val key: (Device) -> Comparable<*>?, val descending: Boolean = false) {
    Name("Name", { it.name.lowercase() }),
    Module("Module", { it.module?.lowercase() }),
    Uptime("Uptime", { it.uptime?.toLong() }, true),
    Rssi("RSSI", { if (it.online == false) null else it.rssi }, true),
    BootCount("BootCount", { it.bootCount }, true),
    RestartReason("RestartReason", { it.restartReason?.lowercase() }),
    LoadAvg("LoadAvg", { it.loadAvg }, true),
    Version("Version", { d -> d.versionShort?.split('.')?.joinToString(".") { it.padStart(5, '0') } }, true),
    Core("Core", { it.core }),
    Ip("IPAddress", { d -> d.ip?.split('.')?.joinToString(".") { it.padStart(3, '0') } }),
    Ssid("SSId", { it.ssid?.lowercase() }),
    Ap("AP", { it.ap }),
    Channel("Channel", { it.channel }),
    Topic("Topic", { it.topic.lowercase() }),
}

@Suppress("UNCHECKED_CAST")
fun List<Device>.sortedFor(sort: SortKey): List<Device> = sortedWith { a, b ->
    val x = sort.key(a) as Comparable<Any>?
    val y = sort.key(b) as Comparable<Any>?
    val r = when {
        x == null && y == null -> 0
        x == null -> 1
        y == null -> -1
        sort.descending -> y.compareTo(x)
        else -> x.compareTo(y)
    }
    if (r != 0) r else a.name.lowercase().compareTo(b.name.lowercase())
}

fun Device.matches(query: String): Boolean {
    val q = query.trim().lowercase()
    if (q.isEmpty()) return true
    return listOf(name, topic, ip, hostname, module).any { it?.lowercase()?.contains(q) == true }
}

val Device.isOffline get() = online == false
/** "Offline" (LWT) or "No answer" (did not answer the status query). */
val Device.offlineLabel get() = when {
    upgrading -> "Upgrading, restarting…"
    noAnswer -> "No answer"
    else -> "Offline"
}
val Device.upgradeBusy get() = upgradeState in setOf("sent", "progress", "restarting")

// ------------------------------------------------------------------ formatting

/** "13d 02:15:46" */
fun fmtDuration(s: ULong?): String {
    if (s == null) return ""
    val t = s.toLong()
    val d = t / 86400
    val h = t % 86400 / 3600
    val m = t % 3600 / 60
    val x = t % 60
    return (if (d > 0) "${d}d " else "") + "%02d:%02d:%02d".format(h, m, x)
}

/** "13d 2h", "5h 12m", "4m" */
fun fmtShort(s: ULong?): String {
    if (s == null) return ""
    val t = s.toLong()
    val d = t / 86400
    val h = t % 86400 / 3600
    val m = t % 3600 / 60
    return when {
        d > 0 -> "${d}d ${h}h"
        h > 0 -> "${h}h ${m}m"
        else -> "${m}m"
    }
}
