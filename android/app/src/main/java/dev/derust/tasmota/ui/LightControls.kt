package dev.derust.tasmota.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.derust.tasmota.core.Light
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.roundToLong
import kotlin.math.sin

/** "#RRGGBB" → Color. */
fun swatchColor(hex: String): Color =
    runCatching { Color(0xFF000000 or hex.removePrefix("#").toLong(16)) }.getOrDefault(Color.Gray)

/**
 * Colour wheel (hue counter-clockwise from red at the right, saturation from the centre, like
 * the desktop and web apps) and sliders. Values are sent when a tap or a drag ends.
 */
@Composable
fun LightControls(
    l: Light,
    enabled: Boolean,
    onHsb: (Long, Long, Long) -> Unit,
    onDimmer: (Long) -> Unit,
    onCt: (Long) -> Unit,
    onWhite: (Long) -> Unit,
) {
    var hue by remember(l.hue) { mutableFloatStateOf((l.hue ?: 0L).toFloat()) }
    var sat by remember(l.sat) { mutableFloatStateOf((l.sat ?: 0L).toFloat()) }
    var dimmer by remember(l.dimmer) { mutableFloatStateOf((l.dimmer ?: 100L).toFloat()) }
    var ct by remember(l.ct) { mutableFloatStateOf((l.ct ?: 326L).toFloat()) }
    var white by remember(l.white) { mutableFloatStateOf((l.white ?: 0L).toFloat()) }

    Column(Modifier.fillMaxWidth()) {
        Text("${l.kind} light", fontWeight = FontWeight.SemiBold)
        Spacer(Modifier.height(8.dp))
        if (l.hasColor) {
            val sendHsb = { onHsb(hue.roundToLong(), sat.roundToLong(), dimmer.roundToLong()) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.Center) {
                Canvas(
                    Modifier
                        .size(220.dp)
                        .pointerInput(enabled) {
                            fun pick(o: Offset) {
                                val r = size.width / 2f
                                val dx = o.x - r
                                val dy = o.y - r
                                hue = (((atan2(-dy, dx) * 180f / PI.toFloat()) + 360f) % 360f)
                                sat = min(1f, hypot(dx, dy) / r) * 100f
                            }
                            detectTapGestures { if (enabled) { pick(it); sendHsb() } }
                        }
                        .pointerInput(enabled) {
                            detectDragGestures(onDragEnd = { if (enabled) sendHsb() }) { change, _ ->
                                if (enabled) {
                                    val r = size.width / 2f
                                    val dx = change.position.x - r
                                    val dy = change.position.y - r
                                    hue = (((atan2(-dy, dx) * 180f / PI.toFloat()) + 360f) % 360f)
                                    sat = min(1f, hypot(dx, dy) / r) * 100f
                                }
                            }
                        },
                ) {
                    val r = size.minDimension / 2f
                    drawCircle(
                        Brush.sweepGradient(
                            listOf(Color.Red, Color.Magenta, Color.Blue, Color.Cyan, Color.Green, Color.Yellow, Color.Red),
                            center,
                        ),
                        r,
                    )
                    drawCircle(Brush.radialGradient(listOf(Color.White, Color.White.copy(alpha = 0f)), center, r), r)
                    val a = hue * PI.toFloat() / 180f
                    val at = Offset(center.x + cos(a) * r * sat / 100f, center.y - sin(a) * r * sat / 100f)
                    drawCircle(Color.Black, 9.dp.toPx(), at, style = Stroke(1.dp.toPx()))
                    drawCircle(Color.White, 8.dp.toPx(), at, style = Stroke(2.dp.toPx()))
                }
            }
            Spacer(Modifier.height(8.dp))
        }
        LightSlider("Brightness", dimmer, 0f..100f, enabled, { dimmer = it }) { onDimmer(dimmer.roundToLong()) }
        if (l.hasCt) {
            LightSlider("White temperature (cold … warm)", ct, 153f..500f, enabled, { ct = it }) { onCt(ct.roundToLong()) }
        }
        if (l.hasWhite) {
            LightSlider("White", white, 0f..100f, enabled, { white = it }) { onWhite(white.roundToLong()) }
        }
    }
}

@Composable
private fun LightSlider(
    label: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    enabled: Boolean,
    onChange: (Float) -> Unit,
    onDone: () -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
        Text("${value.roundToLong()}", style = MaterialTheme.typography.bodySmall)
    }
    Slider(value = value, onValueChange = onChange, valueRange = range, enabled = enabled, onValueChangeFinished = onDone)
}
