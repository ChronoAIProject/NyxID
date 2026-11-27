import Foundation

/// Process-local monotonic launch records. No timing survives a process or grant.
final class LaunchTimingState {
  private struct Record {
    let id: Int
    let kind: String
    let origin: Double
    var active: Double?
    var frame: Double?
  }
  private let configured: String?
  private var binding: String?
  private var current: Record?
  private var sequence = 0
  private var coldSeen = false
  private var backgrounded = false

  init(configured: String?, saved: String?) {
    self.configured = configured?.range(of: "^[a-f0-9]{64}$", options: .regularExpression) != nil ? configured : nil
    self.binding = saved
  }
  var authorized: Bool { configured != nil && binding == configured }
  var launchID: Int? { authorized ? current?.id : nil }
  var hasFirstFrame: Bool { current?.frame != nil }

  /// Mirroring an already granted choice must not invalidate its initial launch.
  /// Withdrawing and regranting never revives a launch that has already begun.
  @discardableResult func setBinding(_ value: String?) -> Bool {
    let previouslyAuthorized = authorized
    binding = value
    guard value == nil || authorized else { current = nil; return false }
    if !authorized || !previouslyAuthorized { current = nil }
    return true
  }
  private func begin(_ kind: String, at now: Double) {
    current = nil
    guard authorized, now.isFinite, now >= 0 else { return }
    sequence += 1
    current = Record(id: sequence, kind: kind, origin: now)
  }
  func cold(at now: Double) {
    guard !coldSeen else { return }
    coldSeen = true
    begin("cold", at: now)
  }
  func background() { backgrounded = true; current = nil }
  func foreground(at now: Double) {
    guard backgrounded else { return }
    backgrounded = false
    begin("hot", at: now)
  }
  private func elapsed(_ now: Double, from origin: Double) -> Double? {
    let duration = now - origin
    return duration.isFinite && (0...120_000).contains(duration) ? duration : nil
  }
  func active(at now: Double) {
    guard authorized, var record = current, record.active == nil,
      let value = elapsed(now, from: record.origin) else { return }
    record.active = value
    current = record
  }
  func firstFrame(_ id: Int, at now: Double) {
    guard authorized, var record = current, record.id == id, record.frame == nil,
      let value = elapsed(now, from: record.origin) else { return }
    record.frame = value
    current = record
  }
  func milestone(_ id: Int, at now: Double) -> Double? {
    guard authorized, let record = current, record.id == id else { return nil }
    return elapsed(now, from: record.origin)
  }
  func snapshot() -> [String: Any]? {
    guard authorized, let record = current else { return nil }
    var result: [String: Any] = [
      "launch_id": record.id, "startup_kind": record.kind,
      "origin_source": record.kind == "cold" ? "ios_did_finish_launching" : "ios_will_enter_foreground",
    ]
    if let active = record.active {
      result["app_active_ms"] = active
      result["active_source"] = "ios_did_become_active"
    }
    if let frame = record.frame {
      result["first_frame_ms"] = frame
      result["first_frame_source"] = "ios_window_commit_then_display_link_proxy"
    }
    return result
  }
}
