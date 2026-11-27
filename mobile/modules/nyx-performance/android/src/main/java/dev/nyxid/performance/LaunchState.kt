package dev.nyxid.performance

/** Native process-local launch state. Tokens and launch IDs never leave the bridge. */
internal class LaunchState(private val now: () -> Double, private val processOrigin: () -> Double) {
  private data class Launch(val id: Long, val kind: String, val origin: Double, var active: Double? = null, var frame: Double? = null, var continuation: Boolean = false)
  private var activity: Any? = null
  private var observedActivity = false
  private var stopped = false
  private var active = false
  private var sequence = 0L
  private var launch: Launch? = null

  fun seed(token: Any?) {
    activity = token; observedActivity = true; stopped = false; active = false; launch = null
  }
  fun bootstrapIdle() {
    // The native process completed a message-loop idle with no activity. Its
    // later first UI is prewarmed and must not use process birth as cold time.
    if (activity == null) observedActivity = true
  }
  fun created(token: Any) {
    if (activity === token) return
    activity = token; stopped = false; active = false
    launch = if (!observedActivity) Launch(++sequence, "cold", processOrigin()) else null
    observedActivity = true
  }
  fun started(token: Any) {
    if (activity === token && stopped) {
      launch = Launch(++sequence, "hot", now()); stopped = false
    }
  }
  fun resumed(token: Any) {
    if (activity !== token) return
    active = true
    launch?.let { if (it.active == null) it.active = elapsed(it.origin) }
  }
  fun paused(token: Any) {
    if (activity === token) { active = false; launch?.continuation = true }
  }
  fun stopped(token: Any, mayReturn: Boolean) {
    // A grant can hydrate before React exposes currentActivity. The first real
    // stop still establishes the existing activity for its next hot return.
    if (activity == null && observedActivity) activity = token
    if (activity !== token) return
    active = false; stopped = mayReturn; launch = null
  }
  fun destroyed(token: Any) {
    if (activity !== token) return
    activity = null; active = false; stopped = false; launch = null
  }
  fun invalidate() { active = false; stopped = false; launch = null }
  fun hasLaunch(): Boolean = launch != null
  fun launchId(): Long? = launch?.id
  fun frame(token: Any, intendedVsync: Double?, expectedId: Long? = null): Boolean {
    val current = launch ?: return false
    if (activity !== token || !active || current.frame != null || (expectedId != null && expectedId != current.id) || (intendedVsync != null && intendedVsync < current.origin)) return false
    current.frame = elapsed(current.origin) ?: return false
    return true
  }
  private fun elapsed(origin: Double): Double? = (now() - origin).takeIf { it.isFinite() && it in 0.0..120000.0 }
  fun milestone(id: Long): Double? = launch?.takeIf { active && it.id == id }?.let { elapsed(it.origin) }
  fun snapshot(): Map<String, Any>? {
    val current = launch?.takeIf { active } ?: return null
    return buildMap {
      put("launch_id", current.id)
      put("startup_kind", current.kind)
      if (current.continuation) put("continuation_only", true)
      put("origin_source", if (current.kind == "cold") "android_process_uptime" else "android_activity_started_after_background")
      current.active?.let { put("app_active_ms", it); put("active_source", "android_activity_resumed") }
      current.frame?.let { put("first_frame_ms", it); put("first_frame_source", "android_window_frame_metrics_callback") }
    }
  }
}
