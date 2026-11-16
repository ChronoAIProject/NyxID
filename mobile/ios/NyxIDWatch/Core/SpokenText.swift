import Foundation

/// Agent replies are Markdown. A watch screen and a speech voice both need
/// plain sentences: no fences, link syntax, emphasis markers or raw URLs.
enum SpokenText {
  /// Characters of a reply shown (and read aloud) before "More".
  static let briefLimit = 240

  struct Brief: Equatable {
    let text: String
    let truncated: Bool
  }

  /// A paragraph shorter than this (a heading such as "Today") takes the
  /// next one along with it.
  static let briefMinimum = 40

  /// The gist of a reply for the watch: its first paragraph (lists and
  /// detail stay behind "More"), cut at a sentence or word if still long.
  static func brief(_ markdown: String, limit: Int = briefLimit) -> Brief {
    let paragraphs = plain(markdown)
      .components(separatedBy: "\n\n")
      .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
      .filter { !$0.isEmpty }
    var taken: [String] = []
    for paragraph in paragraphs {
      taken.append(paragraph)
      if taken.joined(separator: "\n\n").count >= briefMinimum { break }
    }
    var text = taken.joined(separator: "\n\n")
    var truncated = taken.count < paragraphs.count
    if text.count > limit {
      text = cut(text, limit: limit)
      truncated = true
    }
    return Brief(text: text, truncated: truncated)
  }

  static func cut(_ text: String, limit: Int) -> String {
    let head = String(text.prefix(limit))
    if let end = head.lastIndex(where: { ".!?".contains($0) }),
      head.distance(from: head.startIndex, to: end) >= limit / 2
    {
      return String(head[...end])
    }
    if let space = head.lastIndex(where: { $0 == " " || $0 == "\n" }) {
      return String(head[..<space]) + "…"
    }
    return head + "…"
  }

  /// Plain text for the watch screen (keeps line breaks and list items).
  static func plain(_ markdown: String) -> String {
    var lines: [String] = []
    var inFence = false
    for rawLine in markdown.components(separatedBy: "\n") {
      let line = rawLine.trimmingCharacters(in: .whitespaces)
      if line.hasPrefix("```") || line.hasPrefix("~~~") {
        if !inFence { lines.append("[code]") }
        inFence.toggle()
        continue
      }
      if inFence { continue }
      guard let content = stripBlockMarkers(line) else { continue }
      lines.append(inline(content))
    }
    return collapseBlankLines(lines).trimmingCharacters(in: .whitespacesAndNewlines)
  }

  /// What the voice reads: the same brief the screen shows, with code and
  /// URLs named rather than spelled out.
  static func prepare(_ markdown: String) -> String {
    let brief = brief(markdown)
    var text = brief.text.replacingOccurrences(of: "[code]", with: "There's code on screen.")
    text = replacing(pattern: #"https?://[^\s)]+"#, in: text, with: "a link")
    text = text.replacingOccurrences(of: "\n", with: " ")
    text = text.replacingOccurrences(of: "…", with: "")
    text = replacing(pattern: #"\s{2,}"#, in: text, with: " ")
      .trimmingCharacters(in: .whitespaces)
    return brief.truncated ? text + " More on screen." : text
  }

  /// The line without block syntax; nil drops the line entirely.
  private static func stripBlockMarkers(_ line: String) -> String? {
    // A horizontal rule is a paragraph break; a table separator is nothing.
    if line.range(of: #"^(\s*[-*_]\s*){3,}$"#, options: .regularExpression) != nil { return "" }
    if line.range(of: #"^\|?(\s*:?-{2,}:?\s*\|)+\s*:?-*:?\s*\|?$"#, options: .regularExpression) != nil { return nil }
    var line = line
    // Headings and quotes.
    line = replacing(pattern: #"^#{1,6}\s+"#, in: line, with: "")
    line = replacing(pattern: #"^>\s?"#, in: line, with: "")
    // Bullets become a dot; numbered items keep their number.
    line = replacing(pattern: #"^[-*+]\s+(\[[ xX]\]\s+)?"#, in: line, with: "• ")
    if line.hasPrefix("|") {
      line = line
        .trimmingCharacters(in: CharacterSet(charactersIn: "|"))
        .components(separatedBy: "|")
        .map { $0.trimmingCharacters(in: .whitespaces) }
        .joined(separator: ", ")
    }
    return line
  }

  private static func inline(_ line: String) -> String {
    var line = line
    // Images, then links: keep the label.
    line = replacing(pattern: #"!\[([^\]]*)\]\([^)]*\)"#, in: line, with: "$1")
    line = replacing(pattern: #"\[([^\]]+)\]\([^)]*\)"#, in: line, with: "$1")
    // Emphasis, strike-through and inline code markers.
    line = replacing(pattern: #"(\*\*|__)(.+?)\1"#, in: line, with: "$2")
    line = replacing(pattern: #"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])"#, in: line, with: "$1")
    line = replacing(pattern: #"(?<![\w_])_(?!\s)(.+?)(?<!\s)_(?![\w_])"#, in: line, with: "$1")
    line = replacing(pattern: #"~~(.+?)~~"#, in: line, with: "$1")
    line = replacing(pattern: #"`([^`]*)`"#, in: line, with: "$1")
    return line
  }

  private static func collapseBlankLines(_ lines: [String]) -> String {
    var output: [String] = []
    for line in lines {
      if line.isEmpty, output.last?.isEmpty ?? true { continue }
      output.append(line)
    }
    return output.joined(separator: "\n")
  }

  private static func replacing(pattern: String, in text: String, with template: String) -> String {
    guard let regex = try? NSRegularExpression(pattern: pattern) else { return text }
    let range = NSRange(text.startIndex..., in: text)
    return regex.stringByReplacingMatches(in: text, range: range, withTemplate: template)
  }
}
