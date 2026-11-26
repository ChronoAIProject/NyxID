# Native performance measurements

This local Expo module measures application launch milestones and chat frame
timing. The JavaScript consent layer in `src/lib/performance` supplies a valid,
sampled grant before calling `startFrames` and rechecks it before sending the
result.

One native frame observer serves up to two overlapping drag and streaming
windows. JS ends each window within ten seconds. The native observer expires
fifteen seconds after its first window opens. Backgrounding the app, calling
`cancelFrames`, or destroying the module removes the observer.

Android samples `FrameMetrics.TOTAL_DURATION` against `DEADLINE` on API 31+ and
the current display refresh period on older supported versions. Android's
`dropped_callback_count` is an observer-delivery diagnostic, not a dropped frame
count. iOS measures successive `CADisplayLink` timestamps against the previous
callback's requested display period. This is main runloop/display cadence and
cannot claim GPU presentation hitch, frozen-frame or dropped-callback counts.
Budgets follow dynamic refresh rates, with a ten percent tolerance before an
observation counts as over budget. Analyze platforms and sources separately.

The `observed_fps` field counts observed frame callbacks per second of elapsed
window time. Compare windows of the same platform, source and gesture.

Both implementations use a fixed 1,002-bin histogram, one millisecond percentile
precision and a maximum of 10,000 observations per window. Values over one second
share an overflow bin whose percentile uses the window's observed maximum.
Percentiles may round up by less than one millisecond. Do not average window P95s
as a global frame P95. JS projects a closed set of numeric fields and rechecks
consent epoch, visibility and scope before submitting a result.

Launch measurement requires a pre-existing native consent mirror bound to the
build's configured performance destination. The mirror contains only a SHA-256
binding; the module does not transmit it. `setLaunchConsent(null)` invalidates
the current launch immediately. Granting consent during a launch authorizes a
subsequent cold launch or real background-to-foreground transition, without
reviving the launch that was already in progress. Rehydrating the same prior
grant preserves an eligible launch. JavaScript remains responsible for checking
the authoritative saved choice and current consent epoch before transmission.

On iOS, the supported cold origin is `didFinishLaunching`, excluding pre-main
time. A hot origin is `willEnterForeground` following `didEnterBackground`.
App-active uses `didBecomeActive`. The first-frame source is explicitly
`ios_window_commit_then_display_link_proxy`: a normal application window's
layer transaction completes and a later display-link callback occurs. This
proxy does not prove React Native content presentation or GPU completion.
Display observation starts only for a previously authorized launch.

`getLaunchSnapshot` exposes the launch kind, a process-local launch ID and
elapsed milestone values. `measureLaunchMilestone(id)` returns an elapsed value
only for the same authorized launch while the app is active. JavaScript combines
it with the visible, enabled login or chat input and navigation/overlay checks
to measure interaction readiness. Native launch values are bounded to two
minutes and contain no content or absolute timestamps.

The pure native histogram checks under `tests/` cover dynamic 120/60 Hz budgets,
invalid input, observation/time caps, Android callback drops and overflow values.
For Swift, run from `mobile/`:

```sh
xcrun swiftc modules/nyx-performance/ios/FrameHistogram.swift modules/nyx-performance/ios/LaunchTimingState.swift modules/nyx-performance/tests/main.swift -o /tmp/nyx-performance-swift-histogram-tests
/tmp/nyx-performance-swift-histogram-tests
```

For Kotlin, compile `FrameHistogram.kt` and `tests/FrameHistogramTest.kt` together
with the project's Kotlin compiler and stdlib, then run
`dev.nyxid.performance.FrameHistogramTestKt`. Compile the Expo module against
the affected Android library target after autolinking. Full application builds,
device lifecycle checks and Apple framework integration remain separate from
these pure histogram checks.
