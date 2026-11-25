package dev.nyxid.performance

import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import android.view.FrameMetrics
import android.view.ViewTreeObserver
import android.view.Window
import expo.modules.kotlin.Promise
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/** No file/network IO. Observers exist only inside JS-authorized, bounded sessions. */
class NyxPerformanceModule : Module() {
  companion object { private var coldObserved = false }
  private val main = Handler(Looper.getMainLooper())
  private val sessions = LinkedHashMap<String, FrameHistogram>()
  private var attached: Window? = null
  private var foreground = true
  private var resumeAt: Double? = null
  private var generation = 0
  private val pendingReady = mutableSetOf<() -> Unit>()
  private val expires = Runnable { cancelAll() }
  private fun now() = SystemClock.uptimeMillis().toDouble()
  private val listener = Window.OnFrameMetricsAvailableListener { window, metrics, dropped ->
    if (foreground) {
      val refreshBudget = 1000.0 / (window.decorView.display?.refreshRate?.takeIf { it > 0 } ?: 60f)
      val deadline = if (Build.VERSION.SDK_INT >= 31) metrics.getMetric(FrameMetrics.DEADLINE) / 1_000_000.0 else 0.0
      val duration = metrics.getMetric(FrameMetrics.TOTAL_DURATION) / 1_000_000.0
      for (histogram in sessions.values) histogram.add(duration, if (deadline > 0) deadline else refreshBudget, dropped)
    }
  }
  private fun cancelAll() {
    sessions.clear()
    attached?.removeOnFrameMetricsAvailableListener(listener)
    attached = null
    main.removeCallbacks(expires)
  }
  private fun pause() {
    foreground = false
    generation++
    resumeAt = null
    cancelAll()
    cancelReadiness()
  }
  private fun cancelReadiness() {
    pendingReady.toList().forEach { it() }
    pendingReady.clear()
  }
  private fun ready(kind: String, promise: Promise) {
    val activity = appContext.currentActivity
    val start = if (kind == "cold") Process.getStartUptimeMillis().toDouble() else if (kind == "resume") resumeAt else null
    val source = if (kind == "cold") "android_process_uptime" else "android_activity_resume"
    if (!foreground || start == null || activity == null || (kind == "cold" && coldObserved)) { promise.resolve(null); return }
    if (kind == "cold") coldObserved = true
    else resumeAt = null
    val view = activity.window.decorView
    val observer = view.viewTreeObserver
    val cycle = generation
    var done = false
    lateinit var onDraw: ViewTreeObserver.OnDrawListener
    lateinit var timeout: Runnable
    val finish: (Boolean) -> Unit = { drawn ->
      if (!done) {
        done = true
        main.removeCallbacks(timeout)
        view.post { if (observer.isAlive) observer.removeOnDrawListener(onDraw) }
        val elapsed = now() - start
        promise.resolve(if (drawn && foreground && cycle == generation && elapsed in 0.0..120000.0)
          mapOf("duration_ms" to elapsed, "startup_source" to source) else null)
      }
    }
    val cancel = { finish(false) }
    onDraw = ViewTreeObserver.OnDrawListener { view.post { finish(true); pendingReady.remove(cancel) } }
    timeout = Runnable { finish(false); pendingReady.remove(cancel) }
    pendingReady.add(cancel)
    observer.addOnDrawListener(onDraw)
    view.invalidate()
    main.postDelayed(timeout, 1000)
  }
  override fun definition() = ModuleDefinition {
    Name("NyxPerformance")
    OnActivityEntersForeground {
      main.post {
        foreground = true
        generation++
        resumeAt = now()
      }
    }
    OnActivityEntersBackground { main.post { pause() } }
    OnDestroy { main.post { pause() } }
    AsyncFunction("measureReady") { kind: String, promise: Promise -> ready(kind, promise) }.runOnQueue(Queues.MAIN)
    AsyncFunction("startFrames") { token: String ->
      val window = appContext.currentActivity?.window
      if (!foreground || window == null || !token.matches(Regex("^[a-z0-9-]{1,64}$")) || sessions.size >= 2 || sessions.containsKey(token)) false
      else {
        if (attached !== window) {
          cancelAll()
          attached = window
          window.addOnFrameMetricsAvailableListener(listener, main)
        }
        sessions[token] = FrameHistogram(now())
        if (sessions.size == 1) main.postDelayed(expires, 15000)
        true
      }
    }.runOnQueue(Queues.MAIN)
    AsyncFunction("stopFrames") { token: String ->
      val value = sessions.remove(token)?.result(now())?.plus("measurement_source" to "android_frame_metrics")
      if (sessions.isEmpty()) cancelAll()
      value
    }.runOnQueue(Queues.MAIN)
    AsyncFunction("cancelFrames") { cancelAll(); cancelReadiness() }.runOnQueue(Queues.MAIN)
  }
}
