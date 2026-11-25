package dev.nyxid.performance

fun main() {
  val histogram = FrameHistogram(100.0)
  histogram.add(10.0, 1000.0 / 120, 2)
  histogram.add(16.0, 1000.0 / 60, 3)
  histogram.add(Double.NaN, 16.0)
  histogram.add(20.0, 0.0)
  val result = histogram.result(400.0)!!
  check(result["sample_count"] == 2)
  check(result["over_budget_count"] == 1)
  check(result["interval_p50_ms"] == 10.0)
  check(result["interval_p95_ms"] == 16.0)
  check(result["dropped_callback_count"] == 5)
  check(histogram.result(15101.0) == null)
  val bounded = FrameHistogram(0.0)
  repeat(12000) { bounded.add(8.33, 1000.0 / 120) }
  check(bounded.result(10000.0)!!["sample_count"] == 10000)
  check(bounded.result(10000.0)!!["over_budget_count"] == 0)
  val overflow = FrameHistogram(0.0)
  overflow.add(1400.0, 16.0)
  overflow.add(1300.0, 16.0)
  check(overflow.result(2000.0)!!["interval_p95_ms"] == 1400.0)
  println("PASS: Kotlin histogram honors dynamic budgets, invalid values, bounded counts and overflow percentiles")
}
