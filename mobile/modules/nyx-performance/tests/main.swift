import Foundation

let histogram = FrameHistogram(started: 100)
// 120 Hz work becomes over-budget at 10 ms; 60 Hz work at 16 ms is healthy.
histogram.add(milliseconds: 10, budget: 1000 / 120)
histogram.add(milliseconds: 16, budget: 1000 / 60)
histogram.add(milliseconds: .nan, budget: 16)
histogram.add(milliseconds: 20, budget: 0)
let result = histogram.result(now: 400)!
precondition(result["sample_count"] as? Int == 2)
precondition(result["over_budget_count"] as? Int == 1)
precondition(result["interval_p50_ms"] as? Double == 10)
precondition(result["interval_p95_ms"] as? Double == 16)
precondition(result["dropped_callback_count"] == nil)
precondition(histogram.result(now: 15101) == nil)
let bounded = FrameHistogram(started: 0)
for _ in 0..<12000 { bounded.add(milliseconds: 8.33, budget: 1000 / 120) }
precondition(bounded.result(now: 10000)?["sample_count"] as? Int == 10000)
precondition(bounded.result(now: 10000)?["over_budget_count"] as? Int == 0)
let overflow = FrameHistogram(started: 0)
overflow.add(milliseconds: 1400, budget: 16)
overflow.add(milliseconds: 1300, budget: 16)
precondition(overflow.result(now: 2000)?["interval_p95_ms"] as? Double == 1400)

// Delivered callbacks are counted over the explicit wall-time observation
// window, independently of the interval histogram or nominal screen rate.
let cadence = FrameHistogram(started: 100)
for _ in 0..<30 { cadence.recordCallback() }
cadence.add(milliseconds: 8.33, budget: 1000 / 120)
cadence.add(milliseconds: 50, budget: 1000 / 120)
precondition(cadence.result(now: 1100)?["observed_fps"] as? Double == 30)
precondition(cadence.result(now: 1100)?["frame_count"] as? Int == 30)
precondition(cadence.result(now: 1100)?["window_ms"] as? Double == 1000)
precondition(cadence.result(now: 2100)?["observed_fps"] as? Double == 15)
precondition(histogram.result(now: 400)?["observed_fps"] == nil)
// Read the real host process high-water mark. No fixed or invented RSS value.
let actualPeak = peakProcessResidentBytes()
precondition(actualPeak != nil && actualPeak! > 0)
let reportedPeak = cadence.result(now: 1100)?["peak_rss_bytes"] as? Int
precondition(reportedPeak != nil && reportedPeak! >= actualPeak!)
precondition(cadence.result(now: 1100)?["peak_rss_source"] as? String == "ios_getrusage_maxrss")
print("PASS: Swift histogram covers dynamic budgets, bounds, actual callback cadence and process peak RSS")
