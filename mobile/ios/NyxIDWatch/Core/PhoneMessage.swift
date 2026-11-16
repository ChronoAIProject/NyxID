import Foundation

/// Messages the NyxID iPhone app sends over WatchConnectivity. Every value is
/// a string; the iPhone side lives in `mobile/modules/nyx-watch` and
/// `mobile/src/lib/watch/watchPairing.ts`. Keep both in step.
enum PhoneMessage: Equatable {
  /// Sign the watch in by redeeming a one-time login code against `apiBaseURL`.
  /// `match` is the setup code the watch is showing; `frontendURL` lets the
  /// watch hand a chat over to the iPhone.
  case pair(code: String, apiBaseURL: URL, match: String, frontendURL: URL?, expiresAt: Date?)
  /// The person disconnected the watch from the iPhone app.
  case signOut

  static let version = "1"

  init?(_ payload: [String: Any]) {
    guard (payload["v"] as? String) == PhoneMessage.version,
      let kind = payload["kind"] as? String
    else { return nil }
    switch kind {
    case "pair":
      guard let code = (payload["code"] as? String)?.trimmingCharacters(in: .whitespaces),
        PhoneMessage.isPlausibleCode(code),
        let raw = payload["api_base_url"] as? String,
        let url = PhoneMessage.trustedAPIBaseURL(raw),
        let match = payload["match"] as? String,
        match.count == 6, match.allSatisfy(\.isASCIIDigit)
      else { return nil }
      let frontend = (payload["frontend_url"] as? String).flatMap(PhoneMessage.trustedAPIBaseURL)
      let expiresAt = (payload["expires_at"] as? String).flatMap(WatchJSON.parseDate)
      self = .pair(code: code, apiBaseURL: url, match: match, frontendURL: frontend, expiresAt: expiresAt)
    case "sign_out":
      self = .signOut
    default:
      return nil
    }
  }

  /// A login code is short and printable; anything else is not ours.
  static func isPlausibleCode(_ code: String) -> Bool {
    (4...64).contains(code.count)
      && code.unicodeScalars.allSatisfy { scalar in
        CharacterSet.alphanumerics.contains(scalar) || scalar == "-"
      }
  }

  /// HTTPS anywhere; plain HTTP only for loopback development servers.
  /// (Used for both the API and the web app origin.)
  static func trustedAPIBaseURL(_ raw: String) -> URL? {
    guard var components = URLComponents(string: raw),
      let scheme = components.scheme?.lowercased(),
      let host = components.host?.lowercased(), !host.isEmpty,
      components.user == nil, components.password == nil,
      components.query == nil, components.fragment == nil
    else { return nil }
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "::1"
    guard scheme == "https" || (scheme == "http" && loopback) else { return nil }
    while components.path.hasSuffix("/") {
      components.path.removeLast()
    }
    return components.url
  }
}

extension Character {
  var isASCIIDigit: Bool { isASCII && isNumber }
}
