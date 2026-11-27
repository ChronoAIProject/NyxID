package dev.nyxid.performance

import kotlin.math.ceil

/** Fixed 1002-bin histogram, per-frame O(1), independent of content and refresh rate. */
internal class FrameHistogram(val startedMs: Double) {
  private val bins = IntArray(1002)
  var count = 0; private set
  private var overBudget = 0
  private var dropped = 0
  private var maximum = 0.0
  fun add(milliseconds: Double, budgetMs: Double, droppedCallbacks: Int = 0) {
    if (!milliseconds.isFinite() || milliseconds < 0 || !budgetMs.isFinite() || budgetMs <= 0 || count >= 10000) return
    bins[ceil(milliseconds).toInt().coerceIn(0, 1001)]++
    count++
    maximum = maxOf(maximum, milliseconds)
    if (milliseconds > budgetMs * 1.1) overBudget++
    dropped += droppedCallbacks.coerceIn(0, 10000)
  }
  private fun percentile(fraction: Double): Double {
    val rank = ceil(count * fraction).toInt()
    var sum = 0
    for (index in bins.indices) {
      sum += bins[index]
      if (sum >= rank) return if (index == 1001) maximum else index.toDouble()
    }
    return maximum
  }
  fun result(nowMs: Double): Map<String, Any>? {
    if (count < 2 || nowMs - startedMs !in 1.0..15000.0) return null
    return mapOf(
      "duration_ms" to nowMs - startedMs, "sample_count" to count,
      // Observed rendered-window callbacks per elapsed second. TOTAL_DURATION
      // is work time and must never be inverted to estimate display FPS.
      "observed_fps" to count * 1000.0 / (nowMs - startedMs),
      "interval_p50_ms" to percentile(0.50), "interval_p95_ms" to percentile(0.95),
      "interval_max_ms" to maximum, "over_budget_count" to overBudget,
      "dropped_callback_count" to dropped
    )
  }
}
