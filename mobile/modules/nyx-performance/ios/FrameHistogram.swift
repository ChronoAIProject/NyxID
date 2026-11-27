import Foundation

/// Fixed-memory, 1ms histogram. No per-frame array growth or content collection.
final class FrameHistogram {
  let started: Double
  private var bins = [Int](repeating: 0, count: 1002)
  private(set) var count = 0
  private var callbackCount = 0
  private var maximum = 0.0
  private var overBudget = 0
  init(started: Double) { self.started = started }
  /// Called once for each display callback delivered while this window exists.
  /// Count callbacks separately from valid adjacent-timestamp intervals.
  func recordCallback() { callbackCount += 1 }
  func add(milliseconds: Double, budget: Double) {
    guard milliseconds.isFinite, milliseconds >= 0, budget.isFinite, budget > 0, count < 10000 else { return }
    bins[Int(min(1001, ceil(milliseconds)))]+=1
    count+=1
    maximum = max(maximum, milliseconds)
    if milliseconds > budget * 1.1 { overBudget+=1 }
  }
  private func percentile(_ fraction: Double) -> Double {
    let rank = Int(ceil(Double(count) * fraction))
    var sum = 0
    for (index, value) in bins.enumerated() {
      sum+=value
      if sum >= rank { return index == 1001 ? maximum : Double(index) }
    }
    return maximum
  }
  func result(now: Double) -> [String: Any]? {
    guard count >= 2, (1...15000).contains(now - started) else { return nil }
    var result: [String: Any] = ["duration_ms": now - started, "sample_count": count,
      "interval_p50_ms": percentile(0.5), "interval_p95_ms": percentile(0.95),
      "interval_max_ms": maximum, "over_budget_count": overBudget]
    if callbackCount > 0 {
      result["observed_fps"] = Double(callbackCount) * 1000 / (now - started)
      result["frame_count"] = callbackCount
      result["window_ms"] = now - started
    }
    return result
  }
}
