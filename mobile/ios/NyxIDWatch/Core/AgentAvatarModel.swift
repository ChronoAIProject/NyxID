import Foundation

/// Agent avatar identity, matching the iPhone app (`mobile/src/components/
/// agentAvatarModel.ts`) and web (`frontend/src/lib/assistant/nyxbot-avatar.ts`):
/// NyxBot wears the NyxID mark; every other agent gets initials in a muted
/// colour chosen from its id, so an agent looks the same on every device.
enum AgentAvatarModel {
  struct RGBA: Equatable {
    let red: Double
    let green: Double
    let blue: Double
    let alpha: Double

    init(_ red: Double, _ green: Double, _ blue: Double, _ alpha: Double = 1) {
      self.red = red / 255
      self.green = green / 255
      self.blue = blue / 255
      self.alpha = alpha
    }
  }

  struct Tint: Equatable {
    let border: RGBA
    let fill: RGBA
    let text: RGBA
  }

  /// The dark-theme palette (watch screens are always dark).
  static let tints: [Tint] = [
    Tint(border: RGBA(166, 114, 251, 0.30), fill: RGBA(166, 114, 251, 0.15), text: RGBA(166, 114, 251)),
    Tint(border: RGBA(59, 130, 246, 0.30), fill: RGBA(59, 130, 246, 0.10), text: RGBA(59, 130, 246)),
    Tint(border: RGBA(16, 185, 129, 0.30), fill: RGBA(16, 185, 129, 0.10), text: RGBA(16, 185, 129)),
    Tint(border: RGBA(245, 158, 11, 0.30), fill: RGBA(245, 158, 11, 0.10), text: RGBA(245, 158, 11)),
    Tint(border: RGBA(151, 97, 248, 0.30), fill: RGBA(151, 97, 248, 0.15), text: RGBA(184, 148, 251)),
    Tint(border: RGBA(255, 255, 255, 0.15), fill: RGBA(44, 44, 48), text: RGBA(186, 186, 193)),
  ]

  static let nyxBot = Tint(
    border: RGBA(90, 42, 241, 0.30), fill: RGBA(90, 42, 241, 0.10), text: RGBA(90, 42, 241))

  /// FNV-1a over UTF-16 code units, like `agentTintIndex` in TypeScript.
  static func tintIndex(_ id: String) -> Int {
    var hash: UInt32 = 0x811c_9dc5
    for unit in id.utf16 {
      hash ^= UInt32(unit)
      hash = hash &* 0x0100_0193
    }
    return Int(hash % UInt32(tints.count))
  }

  static func tint(for id: String, isNyxBot: Bool) -> Tint {
    isNyxBot ? nyxBot : tints[tintIndex(id)]
  }

  /// Two letters: the first of two words, else the start of one word.
  static func initials(_ name: String) -> String {
    // Same split as `/[^A-Za-z0-9]+/` in TypeScript.
    let asciiAlphanumerics = CharacterSet(
      charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789")
    let words = name
      .components(separatedBy: asciiAlphanumerics.inverted)
      .filter { !$0.isEmpty }
    if words.count >= 2, let first = words[0].first, let second = words[1].first {
      return "\(first)\(second)".uppercased()
    }
    let source = words.first ?? name
    let value = String(source.prefix(2)).uppercased()
    return value.isEmpty ? "?" : value
  }
}
