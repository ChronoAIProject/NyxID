package dev.nyxid.performance

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.MessageQueue
import android.os.Process
import android.os.SystemClock
import android.view.FrameMetrics
import android.view.Window

/** Extra lifecycle/frame listeners exist only under the exact persisted build grant. */
internal object LaunchMonitor {
  private const val BINDING_META = "dev.nyxid.performance.LAUNCH_BINDING"
  private val main = Handler(Looper.getMainLooper())
  private var application: Application? = null
  private var binding: String? = null
  private var registered = false
  private var bootstrapIdle: MessageQueue.IdleHandler? = null
  private val startedActivities = mutableSetOf<Activity>()
  private var observedWindow: Window? = null
  private var observedListener: Window.OnFrameMetricsAvailableListener? = null
  private val state = LaunchState({ SystemClock.uptimeMillis().toDouble() }, { Process.getStartUptimeMillis().toDouble() })
  private val timeout = Runnable { detachFrame() }

  private fun preferences() = application!!.getSharedPreferences("nyx_performance_launch", Context.MODE_PRIVATE)
  private fun allowed(): Boolean = registered && binding != null && preferences().getString("binding", null) == binding
  fun initialize(app: Application) {
    if (application != null) return
    application = app
    binding = runCatching {
      val metadata = app.packageManager.getApplicationInfo(app.packageName, PackageManager.GET_META_DATA).metaData
      (metadata?.get(BINDING_META) as? String)?.takeIf { it.matches(Regex("^[a-f0-9]{64}$")) }
    }.getOrNull()
    if (binding != null && preferences().getString("binding", null) == binding) {
      register()
      val idle = MessageQueue.IdleHandler {
        if (allowed()) state.bootstrapIdle()
        bootstrapIdle = null
        false
      }
      bootstrapIdle = idle
      Looper.myQueue().addIdleHandler(idle)
    }
  }
  private fun register() {
    if (!registered) { registered = true; application!!.registerActivityLifecycleCallbacks(callbacks) }
  }
  fun consent(value: String?, activity: Activity?) {
    val approved = value != null && value == binding
    if (approved && allowed()) return
    // Unregister and invalidate before a durable revoke or a new grant is written.
    detachFrame(); clearBootstrapIdle(); state.seed(null); startedActivities.clear()
    if (registered) application!!.unregisterActivityLifecycleCallbacks(callbacks)
    registered = false
    // commit() can update SharedPreferences' memory even when its disk write
    // fails. Always dirty a revision so a retry really flushes the new grant.
    val revision = runCatching { preferences().getLong("revision", 0) }.getOrDefault(0)
    val written = preferences().edit().apply {
      if (approved) putString("binding", value) else remove("binding")
      putLong("revision", if (revision == Long.MAX_VALUE) 0 else revision + 1)
    }.commit()
    check(written) { "native_launch_consent_write_failed" }
    check(value == null || approved) { "native_launch_consent_binding_mismatch" }
    if (approved) {
      // A grant made after JS mounted cannot recreate a cold launch retroactively.
      state.seed(activity)
      if (activity != null && !activity.isFinishing) startedActivities.add(activity)
      register()
    }
  }
  private fun detachFrame() {
    main.removeCallbacks(timeout)
    val listener = observedListener
    if (listener != null) observedWindow?.removeOnFrameMetricsAvailableListener(listener)
    observedWindow = null; observedListener = null
  }
  private fun clearBootstrapIdle() {
    bootstrapIdle?.let { Looper.myQueue().removeIdleHandler(it) }
    bootstrapIdle = null
  }
  private fun attachFrame(activity: Activity) {
    detachFrame()
    if (!allowed() || !state.hasLaunch()) return
    val launchId = state.launchId() ?: return
    val window = activity.window ?: return
    val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
      // FrameMetrics timestamps share Android's monotonic uptime clock. Reject
      // a queued frame from before this hot/cold origin. The reported boundary
      // is callback delivery, not physical GPU presentation or Apple-style TTFD.
      // INTENDED_VSYNC_TIMESTAMP was added in26. On24/25 the callback is
      // still real and launch-bound; no unsupported metric is read as -1.
      val intended = if (Build.VERSION.SDK_INT >= 26) metrics.getMetric(FrameMetrics.INTENDED_VSYNC_TIMESTAMP) / 1_000_000.0 else null
      if (allowed() && state.frame(activity, intended, launchId)) detachFrame()
    }
    observedWindow = window; observedListener = listener
    window.addOnFrameMetricsAvailableListener(listener, main)
    main.postDelayed(timeout, 15000)
  }
  fun snapshot(): Map<String, Any>? = if (allowed()) state.snapshot() else null
  fun milestone(id: Long): Double? = if (allowed()) state.milestone(id) else null
  fun invalidate() { detachFrame(); clearBootstrapIdle(); state.invalidate() }
  private val callbacks = object : Application.ActivityLifecycleCallbacks {
    override fun onActivityPreCreated(activity: Activity, savedInstanceState: Bundle?) {
      if (activity.componentName.className == application!!.packageName + ".MainActivity") state.created(activity)
    }
    override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
      if (activity.componentName.className == application!!.packageName + ".MainActivity") state.created(activity)
    }
    override fun onActivityStarted(activity: Activity) {
      startedActivities.add(activity)
      if (activity.componentName.className != application!!.packageName + ".MainActivity") return
      state.started(activity); attachFrame(activity)
    }
    override fun onActivityResumed(activity: Activity) {
      if (activity.componentName.className != application!!.packageName + ".MainActivity") return
      state.resumed(activity)
      if (state.snapshot()?.get("first_frame_ms") == null && state.hasLaunch()) attachFrame(activity)
    }
    override fun onActivityPaused(activity: Activity) {
      if (activity.componentName.className != application!!.packageName + ".MainActivity") return
      state.paused(activity); detachFrame()
    }
    override fun onActivityStopped(activity: Activity) {
      startedActivities.remove(activity)
      if (activity.componentName.className != application!!.packageName + ".MainActivity") return
      state.stopped(activity, startedActivities.isEmpty() && !activity.isChangingConfigurations && !activity.isFinishing); detachFrame()
    }
    override fun onActivityDestroyed(activity: Activity) {
      startedActivities.remove(activity)
      if (activity.componentName.className != application!!.packageName + ".MainActivity") return
      state.destroyed(activity); detachFrame()
    }
    override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
  }
}
