import ExpoModulesCore
import QuartzCore
import UIKit

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
  private func now() -> Double { CACurrentMediaTime() * 1000 }
  private func cancel() {
    link?.invalidate(); link = nil; target = nil
    previousTimestamp = nil; previousBudget = nil
    sessions.removeAll()
    expiry?.cancel(); expiry = nil
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
        for histogram in self.sessions.values { histogram.recordCallback() }
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
  public func definition() -> ModuleDefinition {
    Name("NyxPerformance")
    AsyncFunction("setLaunchConsent") { (binding: String?) in try LaunchPerformance.shared.setConsent(binding) }.runOnQueue(.main)
    AsyncFunction("getLaunchSnapshot") { () -> [String: Any]? in LaunchPerformance.shared.snapshot() }.runOnQueue(.main)
    AsyncFunction("measureLaunchMilestone") { (id: Int) -> Double? in LaunchPerformance.shared.milestone(id) }.runOnQueue(.main)
    OnAppEntersBackground { DispatchQueue.main.async { self.cancel() } }
    OnDestroy { DispatchQueue.main.async { self.cancel() } }
    AsyncFunction("startFrames") { (token: String) -> Bool in self.start(token) }.runOnQueue(.main)
    AsyncFunction("stopFrames") { (token: String) -> [String: Any]? in
      let value = self.sessions.removeValue(forKey: token)?.result(now: self.now())
      if self.sessions.isEmpty { self.cancel() }
      return value.map { $0.merging(["measurement_source": "ios_display_link"]) { _, new in new } }
    }.runOnQueue(.main)
    AsyncFunction("cancelFrames") { self.cancel() }.runOnQueue(.main)
  }
}
