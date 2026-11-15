package dev.derust.tasmota.ui

import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/** Line icons on a 24×24 grid, drawn like the web page's (tinted by Icon()). */
private fun lineIcon(name: String, vararg paths: String): ImageVector =
    ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
        for (d in paths) {
            addPath(
                pathData = PathParser().parsePathString(d).toNodes(),
                stroke = SolidColor(Color.Black),
                strokeLineWidth = 1.8f,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
    }.build()

object Icons {
    val Home = lineIcon("home", "M3 11l9-7 9 7", "M5 10v10h14V10", "M10 20v-6h4v6")
    val Health = lineIcon("health", "M3 12h4l3-8 4 16 3-8h4")
    val Firmware = lineIcon(
        "firmware",
        "M8 6h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z",
        "M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4",
    )
    val Wifi = lineIcon(
        "wifi",
        "M2 9a15 15 0 0 1 20 0", "M5 12.5a10 10 0 0 1 14 0", "M8.5 16a5 5 0 0 1 7 0",
        "M12 19.5h0.01",
    )
    val Mqtt = lineIcon("mqtt", "M4 7h13l-3-3", "M20 17H7l3 3")
    val Refresh = lineIcon("refresh", "M20 11a8 8 0 1 0-2.3 5.7", "M20 4v7h-7")
    val Settings = lineIcon(
        "settings",
        "M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1",
        "M15 4v4M9 10v4M17 16v4",
    )
    val Search = lineIcon("search", "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z", "M20 20l-4-4")
    val Close = lineIcon("close", "M6 6l12 12M18 6L6 18")
    val Back = lineIcon("back", "M15 5l-7 7 7 7")
    val Sort = lineIcon("sort", "M7 4v16M3 16l4 4 4-4", "M17 20V4M13 8l4-4 4 4")
    val Lock = lineIcon(
        "lock",
        "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2z",
        "M8 11V7a4 4 0 0 1 8 0v4",
    )
    val Unlock = lineIcon(
        "unlock",
        "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2z",
        "M8 11V7a4 4 0 0 1 7.9-0.9",
    )
    val External = lineIcon("external", "M14 4h6v6", "M20 4l-9 9", "M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5")
}
