package dev.nyxid.performance

import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.FrameMetrics
import android.view.Window
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/** No network or persistence. Observers and bounded procfs reads require a JS-authorized session. */
class NyxPerformanceModule : Module() {
  private val main = Handler(Looper.getMainLooper())
  private val sessions = LinkedHashMap<String, FrameHistogram>()
  private var attached: Window? = null
  private var foreground = true
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
    cancelAll()
  }
  override fun definition() = ModuleDefinition {
    Name("NyxPerformance")
    OnActivityEntersForeground {
      main.post {
        foreground = true
      }
    }
    OnActivityEntersBackground { main.post { pause() } }
    OnDestroy { main.post { pause(); LaunchMonitor.invalidate() } }
    AsyncFunction("setLaunchConsent") { binding: String? ->
      LaunchMonitor.consent(binding, appContext.currentActivity)
    }.runOnQueue(Queues.MAIN)
    AsyncFunction("getLaunchSnapshot") { LaunchMonitor.snapshot() }.runOnQueue(Queues.MAIN)
    AsyncFunction("measureLaunchMilestone") { launchId: Long -> LaunchMonitor.milestone(launchId) }.runOnQueue(Queues.MAIN)
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
      var value = sessions.remove(token)?.result(now())?.plus("measurement_source" to "android_frame_metrics")
      if (value != null) {
        // Read only when an authorized frame window has a usable summary.
        // VmHWM belongs to the process lifetime, including earlier allocations.
        val peakRss = readPeakRssBytes()
        if (peakRss != null) value = value + mapOf(
          "peak_rss_bytes" to peakRss,
          "peak_rss_source" to "android_proc_status_vmhwm"
        )
      }
      if (sessions.isEmpty()) cancelAll()
      value
    }.runOnQueue(Queues.MAIN)
    AsyncFunction("cancelFrames") { cancelAll() }.runOnQueue(Queues.MAIN)
  }
}
