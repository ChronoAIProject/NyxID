import ExpoModulesCore
import QuartzCore
import UIKit

private final class LaunchDisplayCallback: NSObject {
  var callback: ((CADisplayLink) -> Void)?
  @objc func tick(_ link: CADisplayLink) { callback?(link) }
}

/// Installed at didFinishLaunching, before the scene creates its application
/// window. Observers that inspect display callbacks require a pre-existing grant.
final class LaunchPerformance {
  static let shared = LaunchPerformance()
  private let preference = "nyxid.performance.launch-binding.v1"
  private let defaults = UserDefaults.standard
  private let state: LaunchTimingState
  private var durableBinding: String?
  private var durableStateKnown = true
  private var notifications: [NSObjectProtocol] = []
  private var windowNotification: NSObjectProtocol?
  private var displayLink: CADisplayLink?
  private var displayTarget: LaunchDisplayCallback?
  private var proxyLaunch: Int?
  private func now() -> Double { CACurrentMediaTime() * 1000 }

  private init() {
    let saved = UserDefaults.standard.string(forKey: "nyxid.performance.launch-binding.v1")
    state = LaunchTimingState(
      configured: Bundle.main.object(forInfoDictionaryKey: "NyxPerformanceLaunchBinding") as? String,
      saved: saved)
    durableBinding = saved
    let center = NotificationCenter.default
    notifications.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
      guard let self else { return }
      self.state.background(); self.stopProxy(); self.proxyLaunch = nil
      self.updateWindowObservation()
    })
    notifications.append(center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { [weak self] _ in
      guard let self else { return }
      self.state.foreground(at: self.state.authorized ? self.now() : 0)
      self.updateWindowObservation()
      self.observeApplicationWindow()
    })
    notifications.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
      guard let self, self.state.launchID != nil else { return }
      self.state.active(at: self.now())
      self.observeApplicationWindow()
    })
  }
  func didFinishLaunching() {
    // Lifecycle bookkeeping must remember that cold launch has begun even when
    // unauthorized, without reading the performance clock in that state.
    state.cold(at: state.authorized ? now() : 0)
    updateWindowObservation()
  }
  private func updateWindowObservation() {
    if state.launchID != nil && !state.hasFirstFrame {
      guard windowNotification == nil else { return }
      windowNotification = NotificationCenter.default.addObserver(forName: UIWindow.didBecomeVisibleNotification, object: nil, queue: .main) { [weak self] notification in
        self?.observeApplicationWindow(notification.object as? UIWindow)
      }
    } else if let windowNotification {
      NotificationCenter.default.removeObserver(windowNotification)
      self.windowNotification = nil
    }
  }
  private func stopProxy() {
    displayLink?.invalidate(); displayLink = nil
    displayTarget?.callback = nil; displayTarget = nil
  }
  private func applicationWindow(_ candidate: UIWindow?) -> UIWindow? {
    let windows = candidate.map { [$0] } ?? UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }.flatMap { $0.windows }
    return windows.first { window in
      !window.isHidden && window.alpha > 0 && window.windowLevel == .normal &&
        window.bounds.width > 0 && window.bounds.height > 0 &&
        window.windowScene?.session.role == .windowApplication &&
        window.rootViewController?.viewIfLoaded?.window === window
    }
  }
  private func observeApplicationWindow(_ candidate: UIWindow? = nil) {
    guard let id = state.launchID, proxyLaunch != id, !state.hasFirstFrame,
      UIApplication.shared.applicationState != .background,
      let window = applicationWindow(candidate) else { return }
    proxyLaunch = id
    // This is explicitly a window-layer transaction plus a subsequent display
    // callback proxy. It does not assert that RN content reached the display.
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    CATransaction.setCompletionBlock { [weak self, weak window] in
      guard let self, let window, self.state.launchID == id else { return }
      let committedAt = self.now()
      let target = LaunchDisplayCallback()
      target.callback = { [weak self, weak window] tick in
        guard let self, let window else { return }
        guard self.state.launchID == id,
          UIApplication.shared.applicationState != .background,
          self.applicationWindow(window) != nil else { self.stopProxy(); return }
        // The first callback can describe a refresh before the transaction.
        // Only a later display opportunity is eligible for this proxy.
        guard tick.timestamp * 1000 >= committedAt else { return }
        self.state.firstFrame(id, at: tick.timestamp * 1000)
        self.stopProxy()
        self.updateWindowObservation()
      }
      let link = CADisplayLink(target: target, selector: #selector(LaunchDisplayCallback.tick(_:)))
      self.displayTarget = target; self.displayLink = link
      link.add(to: .main, forMode: .common)
      DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
        if self?.proxyLaunch == id { self?.stopProxy() }
      }
    }
    window.layer.setNeedsDisplay()
    CATransaction.commit()
  }
  func setConsent(_ binding: String?) throws {
    guard state.setBinding(binding) else {
      try? persistBinding(nil)
      stopProxy(); updateWindowObservation()
      throw NSError(domain: "NyxPerformance", code: 1, userInfo: [NSLocalizedDescriptionKey: "Launch measurement configuration is unavailable."])
    }
    if binding == nil { stopProxy(); proxyLaunch = nil }
    updateWindowObservation()
    // Hydration and revisiting settings do not synchronously rewrite a mirror
    // that was already read from disk or successfully persisted in this process.
    guard !durableStateKnown || binding != durableBinding else { return }
    try persistBinding(binding)
  }
  private func persistBinding(_ binding: String?) throws {
    if let binding { defaults.set(binding, forKey: preference) }
    else { defaults.removeObject(forKey: preference) }
    guard defaults.synchronize() else {
      durableStateKnown = false
      state.setBinding(nil); defaults.removeObject(forKey: preference); stopProxy()
      updateWindowObservation()
      throw NSError(domain: "NyxPerformance", code: 2, userInfo: [NSLocalizedDescriptionKey: "Launch measurement choice could not be saved."])
    }
    durableBinding = binding; durableStateKnown = true
  }
  func snapshot() -> [String: Any]? {
    UIApplication.shared.applicationState == .background ? nil : state.snapshot()
  }
  func milestone(_ id: Int) -> Double? {
    guard UIApplication.shared.applicationState == .active, state.launchID == id else { return nil }
    return state.milestone(id, at: now())
  }
}

public class NyxPerformanceAppDelegateSubscriber: ExpoAppDelegateSubscriber {
  public func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
    // Public iOS SDK supplies this supported origin. It deliberately excludes
    // pre-main time; no process-creation timestamp is inferred from wall time.
    LaunchPerformance.shared.didFinishLaunching()
    return true
  }
}
