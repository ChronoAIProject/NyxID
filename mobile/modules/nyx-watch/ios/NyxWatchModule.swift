import ExpoModulesCore
import WatchConnectivity

/// The iPhone side of the NyxID Apple Watch link.
///
/// The phone never shares its own session with the watch. It only hands the
/// watch app a one-time login code (minted by the signed-in person) so the
/// watch can redeem its own, separately revocable session.
public class NyxWatchModule: Module {
  public func definition() -> ModuleDefinition {
    Name("NyxWatch")

    Events("onStatusChange")

    OnCreate {
      WatchLink.shared.onChange = { [weak self] status in
        self?.sendEvent("onStatusChange", status)
      }
      WatchLink.shared.activate()
    }

    OnDestroy {
      WatchLink.shared.onChange = nil
    }

    AsyncFunction("getStatus") { () async -> [String: Any] in
      await WatchLink.shared.status()
    }

    /// Delivers a small string-only message to the watch app. Resolves
    /// "delivered" when the watch app acknowledged it immediately, or
    /// "queued" when it will receive it the next time it runs.
    AsyncFunction("send") { (message: [String: String]) async throws -> String in
      try await WatchLink.shared.send(message)
    }

    /// Replaces any queued message with a blank one, so a one-time code
    /// doesn't linger in WatchConnectivity once setup has ended.
    AsyncFunction("clearPending") { () async in
      await WatchLink.shared.clearPending()
    }
  }
}

final class WatchUnsupportedException: Exception {
  override var reason: String { "This device can't pair with an Apple Watch." }
}

final class WatchNotPairedException: Exception {
  override var reason: String { "No Apple Watch is paired with this iPhone." }
}

final class WatchAppNotInstalledException: Exception {
  override var reason: String { "NyxID isn't installed on your Apple Watch yet." }
}

final class WatchLink: NSObject, WCSessionDelegate {
  static let shared = WatchLink()

  var onChange: (([String: Any]) -> Void)?

  private let lock = NSLock()
  private var activationWaiters: [CheckedContinuation<Void, Never>] = []
  /// The setup code the NyxID watch app is showing ("" once it is signed in).
  private var watchCode = ""

  func activate() {
    guard WCSession.isSupported() else { return }
    let session = WCSession.default
    if session.delegate == nil {
      session.delegate = self
    }
    if session.activationState == .notActivated {
      session.activate()
    }
  }

  func status() async -> [String: Any] {
    await waitForActivation()
    return snapshot()
  }

  func send(_ message: [String: String]) async throws -> String {
    guard WCSession.isSupported() else { throw WatchUnsupportedException() }
    await waitForActivation()
    let session = WCSession.default
    guard session.activationState == .activated, session.isPaired else {
      throw WatchNotPairedException()
    }
    guard session.isWatchAppInstalled else { throw WatchAppNotInstalledException() }

    if session.isReachable {
      do {
        return try await withCheckedThrowingContinuation { continuation in
          session.sendMessage(
            message,
            replyHandler: { reply in
              continuation.resume(returning: (reply["status"] as? String) ?? "delivered")
            },
            errorHandler: { error in
              continuation.resume(throwing: error)
            }
          )
        }
      } catch {
        // The watch app went to the background mid-send; fall through to the
        // queued transfer, which the watch reads the next time it runs.
      }
    }

    // Not reachable: leave it as the application context, which the watch
    // app receives when it next runs. Only the newest context is kept, which
    // suits a single-use code: a fresh one replaces an older one.
    do {
      try session.updateApplicationContext(message)
    } catch {
      throw WatchNotPairedException()
    }
    return "queued"
  }

  func clearPending() async {
    guard WCSession.isSupported() else { return }
    await waitForActivation()
    let session = WCSession.default
    guard session.activationState == .activated else { return }
    try? session.updateApplicationContext(["v": "1", "kind": "none"])
  }

  private func snapshot() -> [String: Any] {
    guard WCSession.isSupported() else {
      return [
        "supported": false, "activated": false, "paired": false, "installed": false,
        "reachable": false, "watchCode": "",
      ]
    }
    let session = WCSession.default
    let activated = session.activationState == .activated
    return [
      "supported": true,
      "activated": activated,
      "paired": activated && session.isPaired,
      "installed": activated && session.isWatchAppInstalled,
      "reachable": activated && session.isReachable,
      "watchCode": lock.withLock { watchCode },
    ]
  }

  /// The watch shares its setup code as its application context.
  private func readHello(_ context: [String: Any]) {
    guard (context["v"] as? String) == "1", (context["kind"] as? String) == "hello" else { return }
    let code = (context["code"] as? String) ?? ""
    let valid = code.count == 6 && code.allSatisfy { $0.isASCII && $0.isNumber }
    lock.withLock { watchCode = valid ? code : "" }
  }

  private func waitForActivation() async {
    guard WCSession.isSupported() else { return }
    activate()
    if WCSession.default.activationState != .notActivated { return }
    await withTaskGroup(of: Void.self) { group in
      group.addTask {
        await withCheckedContinuation { continuation in
          self.lock.lock()
          if WCSession.default.activationState != .notActivated {
            self.lock.unlock()
            continuation.resume()
            return
          }
          self.activationWaiters.append(continuation)
          self.lock.unlock()
        }
      }
      group.addTask {
        try? await Task.sleep(nanoseconds: 3_000_000_000)
        if !Task.isCancelled { self.resumeWaiters() }
      }
      await group.next()
      group.cancelAll()
    }
  }

  private func resumeWaiters() {
    lock.lock()
    let waiters = activationWaiters
    activationWaiters.removeAll()
    lock.unlock()
    waiters.forEach { $0.resume() }
  }

  private func notify() {
    onChange?(snapshot())
  }

  // MARK: WCSessionDelegate

  func session(
    _ session: WCSession,
    activationDidCompleteWith activationState: WCSessionActivationState,
    error: Error?
  ) {
    readHello(session.receivedApplicationContext)
    resumeWaiters()
    notify()
  }

  func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
    readHello(applicationContext)
    notify()
  }

  func sessionDidBecomeInactive(_ session: WCSession) {
    notify()
  }

  func sessionDidDeactivate(_ session: WCSession) {
    // The person switched to another watch: reactivate for the new one.
    session.activate()
    notify()
  }

  func sessionWatchStateDidChange(_ session: WCSession) {
    notify()
  }

  func sessionReachabilityDidChange(_ session: WCSession) {
    notify()
  }
}
