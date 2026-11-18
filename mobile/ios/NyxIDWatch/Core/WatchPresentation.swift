import Foundation

/// "now", "5m", "3h", "Yesterday", "Mon", "3 Nov": enough on a watch.
enum RelativeTime {
  static func short(_ date: Date, now: Date = Date(), calendar: Calendar = .current) -> String {
    let seconds = now.timeIntervalSince(date)
    if seconds < 60 { return "now" }
    if seconds < 3_600 { return "\(Int(seconds / 60))m" }
    if calendar.isDate(date, inSameDayAs: now) { return "\(Int(seconds / 3_600))h" }
    if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
      calendar.isDate(date, inSameDayAs: yesterday)
    {
      return "Yesterday"
    }
    let formatter = DateFormatter()
    formatter.calendar = calendar
    formatter.timeZone = calendar.timeZone
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.dateFormat = seconds < 6 * 86_400 ? "EEE" : "d MMM"
    return formatter.string(from: date)
  }
}

extension ApprovalRequest {
  /// "Read your Gmail", or the server's own description when it has one.
  var headline: String {
    if let description = actionDescription?.trimmingCharacters(in: .whitespaces),
      !description.isEmpty, !description.hasPrefix("proxy:"),
      description.range(of: #"^[A-Z]+\s+/"#, options: .regularExpression) == nil
    {
      return description.prefix(1).uppercased() + description.dropFirst()
    }
    switch risk {
    case .low: return "Read your \(title)"
    case .medium: return "Make changes in \(title)"
    case .high: return "Delete in \(title)"
    case .unknown: return "Use \(title)"
    }
  }

  /// "Researcher wants access".
  var askedBy: String {
    let who = requester.prefix(1).uppercased() + requester.dropFirst()
    return "\(who) wants access"
  }

  var accessLabel: String {
    switch risk {
    case .low: return "Read-only"
    case .medium: return "Makes changes"
    case .high: return "Deletes data"
    case .unknown: return "Access"
    }
  }

  /// How long an approval lasts: once, or the person's grant period.
  func durationLabel(grantDays: Int?) -> String {
    guard isGrant else { return "Once" }
    let days = max(1, grantDays ?? 30)
    return days == 1 ? "1 day" : "\(days) days"
  }

  /// "Gmail · Calendar" for a stack of approvals.
  static func servicesSummary(_ approvals: [ApprovalRequest]) -> String {
    var names: [String] = []
    for approval in approvals where !names.contains(approval.title) {
      names.append(approval.title)
    }
    let shown = names.prefix(2).joined(separator: " · ")
    return names.count > 2 ? "\(shown) +\(names.count - 2)" : shown
  }
}

/// The code the watch shows while signed out. The iPhone shows the same code
/// and includes it when it hands over a sign-in, so only this watch, on this
/// screen, can use it.
enum SetupCode {
  static func generate() -> String {
    String(format: "%06d", Int.random(in: 0..<1_000_000))
  }

  /// "482913" → "482 913".
  static func display(_ code: String) -> String {
    guard code.count == 6 else { return code }
    return code.prefix(3) + " " + code.suffix(3)
  }

  /// Whether a sign-in from the iPhone is for the code on screen now.
  static func accepts(_ match: String, showing code: String?) -> Bool {
    guard let code, code.count == 6 else { return false }
    return match == code
  }

  /// What the watch tells the iPhone (application context).
  static func hello(code: String?) -> [String: String] {
    ["v": PhoneMessage.version, "kind": "hello", "code": code ?? ""]
  }
}

/// What a thread shows: the person's messages and the agents' replies,
/// oldest first. Pages merge by id, so a later read replaces an earlier one
/// (a reply that was still streaming, say).
enum Transcript {
  static func merge(_ current: [ChatMessage], _ page: [ChatMessage]) -> [ChatMessage] {
    var byId: [String: ChatMessage] = [:]
    for message in current + page where shows(message) {
      byId[message.id] = message
    }
    return byId.values.sorted { $0.seq < $1.seq }
  }

  static func shows(_ message: ChatMessage) -> Bool {
    if message.isFromPerson { return !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    if message.isReply { return message.failed || !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    return false
  }
}
