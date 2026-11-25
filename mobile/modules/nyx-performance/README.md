# Native performance measurements

This local Expo module supplies bounded measurements to the consent layer in
`src/lib/performance`. It owns no storage, identity, network transport or user
content. Merely loading the module records only lifecycle clock origins. JS must
resolve the persisted launch grant before calling `measureReady`, and obtain a
currently valid sampled consent epoch before calling `startFrames`.

`measureReady("cold")` starts at Android process uptime or the iOS
`didFinishLaunching` callback. `measureReady("resume")` starts at the Android
activity foreground callback or iOS `applicationWillEnterForeground`. The end is
the next native drawing/display opportunity after the actual enabled input has
laid out, or the login page has laid out. This app-defined boundary includes
permission hydration latency when permission resolves after layout. It is not
an OS TTID or TTFD measurement. Inactive-only iOS transitions do not produce
resume measurements. A launch interrupted by a real background is discarded.

`startFrames` admits at most two overlapping tokens. A single native observer
serves both the drag and stream windows. JS ends each sample within ten seconds;
the native observer has a fifteen-second cap from its first window. Background,
explicit cancellation and module destruction remove the observer. Explicit
`cancelFrames` also removes pending readiness observers immediately. Readiness
times out after one second without a drawing/display callback.

Android samples `FrameMetrics.TOTAL_DURATION` against `DEADLINE` on API 31+ and
the current display refresh period on older supported versions. Android's
`dropped_callback_count` is an observer-delivery diagnostic, not a dropped frame
count. iOS measures successive `CADisplayLink` timestamps against the previous
callback's requested display period. This is main runloop/display cadence and
cannot claim GPU presentation hitch, frozen-frame or dropped-callback counts.
Budgets follow dynamic refresh rates, with a ten percent tolerance before an
observation counts as over budget. Analyze platforms and sources separately.

Both implementations use a fixed 1,002-bin histogram, one millisecond percentile
precision and a maximum of 10,000 observations per window. Values over one second
share an overflow bin whose percentile uses the window's observed maximum.
Percentiles may round up by less than one millisecond. Do not average window P95s
as a global frame P95. JS projects a closed set of numeric fields and rechecks
consent epoch, visibility and scope before submitting a result.

The pure native histogram checks under `tests/` cover dynamic 120/60 Hz budgets,
invalid input, observation/time caps, Android callback drops and overflow values.
For Swift, run from `mobile/`:

```sh
xcrun swiftc modules/nyx-performance/ios/FrameHistogram.swift modules/nyx-performance/tests/main.swift -o /tmp/nyx-performance-swift-histogram-tests
/tmp/nyx-performance-swift-histogram-tests
```

For Kotlin, compile `FrameHistogram.kt` and `tests/FrameHistogramTest.kt` together
with the project's Kotlin compiler and stdlib, then run
`dev.nyxid.performance.FrameHistogramTestKt`. Compile the Expo module against
the affected Android library target after autolinking. Full application builds,
device lifecycle checks and Apple framework integration remain separate from
these pure histogram checks.
