import Foundation

/// A reply shaped for the watch: a headline, then the first list (times in
/// their own column), and whether anything else is left for "More".
struct ReplyLayout: Equatable {
  struct Row: Equatable {
    let time: String?
    let text: String
  }

  let lead: String
  let rows: [Row]
  let hasMore: Bool

  static let maxRows = 6

  /// The lead's first sentence, shown bold like a headline.
  var headline: String { Self.splitLead(lead).headline }
  /// The rest of the lead, shown smaller.
  var detail: String? { Self.splitLead(lead).detail }

  static func splitLead(_ lead: String) -> (headline: String, detail: String?) {
    guard let end = lead.range(of: #"[.!?]\s+"#, options: .regularExpression),
      lead.distance(from: lead.startIndex, to: end.lowerBound) < 140
    else { return (lead, nil) }
    let headline = String(lead[...end.lowerBound])
    let rest = lead[end.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
    return (headline, rest.isEmpty ? nil : rest)
  }

  static func make(_ markdown: String) -> ReplyLayout {
    var blocks = SpokenText.plain(markdown)
      .components(separatedBy: "\n\n")
      .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
      .filter { !$0.isEmpty && $0 != "[code]" }
    var leadParts: [String] = []
    while let first = blocks.first, !isList(first) {
      leadParts.append(blocks.removeFirst())
      if leadParts.joined(separator: " ").count >= SpokenText.briefMinimum { break }
    }
    var lead = leadParts.joined(separator: "\n")
    var hasMore = false
    if lead.count > SpokenText.briefLimit {
      lead = SpokenText.cut(lead, limit: SpokenText.briefLimit)
      hasMore = true
    }
    var rows: [Row] = []
    if let first = blocks.first, isList(first) {
      blocks.removeFirst()
      let items = first.components(separatedBy: "\n").compactMap(listItem)
      rows = items.prefix(maxRows).map(row)
      if items.count > maxRows { hasMore = true }
    }
    if !blocks.isEmpty { hasMore = true }
    return ReplyLayout(lead: lead, rows: rows, hasMore: hasMore)
  }

  private static func isList(_ block: String) -> Bool {
    block.components(separatedBy: "\n").allSatisfy { listItem($0) != nil }
  }

  /// "• Lunch" or "2. Lunch" → "Lunch".
  private static func listItem(_ line: String) -> String? {
    guard let range = line.range(of: #"^(•|\d{1,2}[.)])\s+"#, options: .regularExpression) else {
      return nil
    }
    let text = line[range.upperBound...].trimmingCharacters(in: .whitespaces)
    return text.isEmpty ? nil : text
  }

  /// "10:00 Design review" → time "10:00", text "Design review".
  private static func row(_ item: String) -> Row {
    let pattern = #"^(\d{1,2}:\d{2}(\s?[AaPp][Mm])?(\s?[–-]\s?\d{1,2}:\d{2}(\s?[AaPp][Mm])?)?)\s*[:·–-]?\s+"#
    guard let range = item.range(of: pattern, options: .regularExpression) else {
      return Row(time: nil, text: item)
    }
    let time = item[range].trimmingCharacters(in: CharacterSet(charactersIn: " :·–-"))
    return Row(time: time, text: String(item[range.upperBound...]))
  }
}

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
