package dev.nyxid.performance

object LaunchStateTest {
@JvmStatic fun main(args: Array<String>) {
  var clock = 100.0
  var originsRead = 0
  val state = LaunchState({ clock }, { originsRead++; 10.0 })
  check(originsRead == 0)
  val activity = Any()
  state.created(activity); state.started(activity); state.resumed(activity)
  val cold = state.snapshot()!!
  check(originsRead == 1)
  check(cold["startup_kind"] == "cold")
  check(cold["app_active_ms"] == 90.0)
  check(cold["first_frame_ms"] == null)
  clock = 125.0
  check(!state.frame(activity, 5.0)) // a queued pre-origin frame is inadmissible
  check(state.frame(activity, 100.0))
  check(state.snapshot()!!["first_frame_ms"] == 115.0)
  val coldId = cold["launch_id"] as Long
  clock = 150.0
  check(state.milestone(coldId) == 140.0)
  // Permission dialogs pause/resume without an app-background/start transition.
  state.paused(activity); check(state.snapshot() == null); check(state.milestone(coldId) == null)
  clock = 200.0; state.resumed(activity)
  check(state.snapshot() == null) // never replay the prior launch after a pause
  state.stopped(activity, true); check(state.snapshot() == null)
  clock = 2000.0; state.started(activity); clock = 2010.0; state.resumed(activity)
  val hot = state.snapshot()!!
  check(hot["startup_kind"] == "hot" && hot["app_active_ms"] == 10.0)
  check(hot["launch_id"] != coldId && state.milestone(coldId) == null)
  clock = 2020.0; check(state.frame(activity, 2010.0))
  check(state.snapshot()!!["first_frame_ms"] == 20.0)
  // Configuration change/internal activity does not create another hot/cold.
  state.stopped(activity, false); state.destroyed(activity)
  val recreated = Any(); state.created(recreated); state.started(recreated); state.resumed(recreated)
  check(state.snapshot() == null)
  state.stopped(recreated, false); state.started(recreated); state.resumed(recreated)
  check(state.snapshot() == null)
  // Opt-in during an existing activity cannot fabricate a cold origin.
  val granted = LaunchState({ clock }, { error("late grant must not read cold origin") })
  granted.seed(recreated); granted.resumed(recreated); check(granted.snapshot() == null)
  granted.stopped(recreated, true); granted.started(recreated); granted.resumed(recreated)
  check(granted.snapshot()!!["startup_kind"] == "hot")
  // A hot launch not sampled by JS cannot be rediscovered as a later hot cycle.
  val unsampledHot = granted.snapshot()!!["launch_id"] as Long
  granted.paused(recreated); granted.resumed(recreated)
  check(granted.snapshot() == null && granted.milestone(unsampledHot) == null)
  granted.stopped(recreated, true); granted.started(recreated); granted.resumed(recreated)
  val legacyId = granted.snapshot()!!["launch_id"] as Long
  check(!granted.frame(recreated, null, unsampledHot))
  check(granted.frame(recreated, null, legacyId)) // SDK24/25 real callback
  val id = granted.snapshot()!!["launch_id"] as Long
  granted.seed(null); check(granted.snapshot() == null && granted.milestone(id) == null)
  granted.stopped(recreated, true); granted.started(recreated); granted.resumed(recreated)
  check(granted.snapshot()!!["startup_kind"] == "hot")
  println("PASS: native launch lifecycle origins, permission pause, same-activity hot, recreation and revoke fences")
}
}
