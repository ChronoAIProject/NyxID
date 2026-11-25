import ExpoModulesCore
import QuartzCore
import UIKit

// Lifecycle origins contain only process-local monotonic clocks; never persisted.
enum NativePerformanceOrigins {
  static var launch: Double?
  static var resume: Double?
  static var generation = 0
  static var coldObserved = false
}
public class NyxPerformanceAppDelegateSubscriber: ExpoAppDelegateSubscriber {
  public func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
    NativePerformanceOrigins.launch = CACurrentMediaTime() * 1000
    return true
  }
  public func applicationWillEnterForeground(_ application: UIApplication) {
    NativePerformanceOrigins.resume = CACurrentMediaTime() * 1000
    NativePerformanceOrigins.generation+=1
  }
  public func applicationDidEnterBackground(_ application: UIApplication) {
    NativePerformanceOrigins.generation+=1
    NativePerformanceOrigins.resume = nil
  }
}

// CADisplayLink reports main-runloop/display cadence, NOT physical GPU hitches.
private final class DisplayTicks: NSObject {
  var callback: ((CADisplayLink) -> Void)?
  @objc func tick(_ link: CADisplayLink) { callback?(link) }
}
public class NyxPerformanceModule: Module {
  private var sessions: [String: FrameHistogram] = [:]
  private var link: CADisplayLink?
  private var target: DisplayTicks?
  private var previousTimestamp: Double?
  private var previousBudget: Double?
  private var expiry: DispatchWorkItem?
  private var pendingReady: [UUID: () -> Void] = [:]
  private func now() -> Double { CACurrentMediaTime() * 1000 }
  private func cancel() {
    link?.invalidate(); link = nil; target = nil
    previousTimestamp = nil; previousBudget = nil
    sessions.removeAll()
    expiry?.cancel(); expiry = nil
  }
  private func cancelReadiness() {
    for finish in Array(pendingReady.values) { finish() }
    pendingReady.removeAll()
  }
  private func start(_ token: String) -> Bool {
    guard UIApplication.shared.applicationState == .active,
      token.range(of: "^[a-z0-9-]{1,64}$", options: .regularExpression) != nil,
      sessions.count < 2, sessions[token] == nil else { return false }
    sessions[token] = FrameHistogram(started: now())
    if link == nil {
      let target = DisplayTicks()
      target.callback = { [weak self] tick in
        guard let self else { return }
        // targetTimestamp's expected next deadline supports 60/90/120Hz changes.
        let timestamp = tick.timestamp * 1000
        let budget = (tick.targetTimestamp - tick.timestamp) * 1000
        if let previous = self.previousTimestamp, let expected = self.previousBudget {
          for histogram in self.sessions.values where previous >= histogram.started {
            histogram.add(milliseconds: timestamp - previous, budget: expected)
          }
        }
        self.previousTimestamp = timestamp
        self.previousBudget = budget > 0 ? budget : nil
      }
      let link = CADisplayLink(target: target, selector: #selector(DisplayTicks.tick(_:)))
      link.add(to: .main, forMode: .common)
      self.target = target; self.link = link
    }
    if sessions.count == 1 {
      let expiry = DispatchWorkItem { [weak self] in self?.cancel() }
      self.expiry = expiry
      DispatchQueue.main.asyncAfter(deadline: .now() + 15, execute: expiry)
    }
    return true
  }
  private func measure(_ kind: String, promise: Promise) {
    let start = kind == "cold" ? NativePerformanceOrigins.launch : kind == "resume" ? NativePerformanceOrigins.resume : nil
    guard let start, UIApplication.shared.applicationState == .active,
      kind != "cold" || !NativePerformanceOrigins.coldObserved else { promise.resolve(nil); return }
    if kind == "cold" { NativePerformanceOrigins.coldObserved = true }
    else { NativePerformanceOrigins.resume = nil }
    let generation = NativePerformanceOrigins.generation
    // One native display opportunity after RN has reported target input layout.
    // This is an app-defined readiness boundary, not Apple's launch/TTFD metric.
    let ticks = DisplayTicks()
    let token = UUID()
    var finished = false
    var readyLink: CADisplayLink?
    let finish: (Bool) -> Void = { drawn in
      guard !finished else { return }
      finished = true
      self.pendingReady.removeValue(forKey: token)
      readyLink?.invalidate()
      ticks.callback = nil
      let elapsed = CACurrentMediaTime() * 1000 - start
      if drawn && UIApplication.shared.applicationState == .active && generation == NativePerformanceOrigins.generation && (0...120000).contains(elapsed) {
        promise.resolve(["duration_ms": elapsed, "startup_source": kind == "cold" ? "ios_did_finish_launching" : "ios_will_enter_foreground"])
      } else { promise.resolve(nil) }
    }
    ticks.callback = { _ in finish(true) }
    pendingReady[token] = { finish(false) }
    readyLink = CADisplayLink(target: ticks, selector: #selector(DisplayTicks.tick(_:)))
    readyLink?.add(to: .main, forMode: .common)
    DispatchQueue.main.asyncAfter(deadline: .now() + 1) { finish(false) }
  }
  public func definition() -> ModuleDefinition {
    Name("NyxPerformance")
    OnAppEntersBackground { DispatchQueue.main.async { self.cancel(); self.cancelReadiness() } }
    OnDestroy { DispatchQueue.main.async { self.cancel(); self.cancelReadiness() } }
    AsyncFunction("measureReady") { (kind: String, promise: Promise) in self.measure(kind, promise: promise) }.runOnQueue(.main)
    AsyncFunction("startFrames") { (token: String) -> Bool in self.start(token) }.runOnQueue(.main)
    AsyncFunction("stopFrames") { (token: String) -> [String: Any]? in
      let value = self.sessions.removeValue(forKey: token)?.result(now: self.now())
      if self.sessions.isEmpty { self.cancel() }
      return value.map { $0.merging(["measurement_source": "ios_display_link"]) { _, new in new } }
    }.runOnQueue(.main)
    AsyncFunction("cancelFrames") { self.cancel(); self.cancelReadiness() }.runOnQueue(.main)
  }
}
