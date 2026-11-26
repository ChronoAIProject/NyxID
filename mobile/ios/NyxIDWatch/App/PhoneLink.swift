import Foundation
import WatchConnectivity

/// Receives sign-in codes and sign-out requests from the NyxID iPhone app,
/// and tells it which setup code this watch is showing.
final class PhoneLink: NSObject, WCSessionDelegate {
  static let shared = PhoneLink()

  /// Called on the main actor for every recognised message.
  var onMessage: (@MainActor (PhoneMessage) -> Void)?

  private let lock = NSLock()
  private var pendingHello: [String: String]?

  func activate() {
    guard WCSession.isSupported() else { return }
    let session = WCSession.default
    session.delegate = self
    session.activate()
  }

  /// Shares the setup code (nil once signed in) as the application context,
  /// which the iPhone reads whenever it next looks.
  func sayHello(code: String?) {
    let hello = SetupCode.hello(code: code)
    guard WCSession.isSupported() else { return }
    if WCSession.default.activationState == .activated {
      try? WCSession.default.updateApplicationContext(hello)
    } else {
      lock.withLock { pendingHello = hello }
    }
  }

  private func deliver(_ payload: [String: Any]) {
    guard let message = PhoneMessage(payload) else { return }
    Task { @MainActor in
      self.onMessage?(message)
    }
  }

  // MARK: WCSessionDelegate

  func session(
    _ session: WCSession,
    activationDidCompleteWith activationState: WCSessionActivationState,
    error: Error?
  ) {
    guard activationState == .activated, let hello = lock.withLock({ pendingHello }) else { return }
    lock.withLock { pendingHello = nil }
    try? session.updateApplicationContext(hello)
  }

  func session(
    _ session: WCSession,
    didReceiveMessage message: [String: Any],
    replyHandler: @escaping ([String: Any]) -> Void
  ) {
    // Acknowledge at once; the iPhone follows the sign-in through the
    // login code's own status, not through this reply.
    replyHandler(["status": PhoneMessage(message) == nil ? "ignored" : "delivered"])
    deliver(message)
  }

  func session(_ session: WCSession, didReceiveMessage message: [String: Any]) {
    deliver(message)
  }

  /// A code the iPhone left queued while this app wasn't reachable.
  func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
    deliver(applicationContext)
  }

  func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any] = [:]) {
    deliver(userInfo)
  }
}
