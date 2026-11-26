import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Error body every NyxID API error returns (`backend/src/errors/mod.rs`).
struct APIErrorBody: Decodable, Equatable {
  let error: String
  let errorCode: Int
  let message: String
}

enum WatchAPIError: Error, Equatable {
  /// No session, or the server refused the refresh token: sign in again.
  case signedOut
  /// The server answered with an error response.
  case server(status: Int, body: APIErrorBody?)
  /// The request never got an HTTP answer (offline, timeout, TLS).
  case transport(String)
  /// The server answered with a shape this app doesn't understand.
  case decoding(String)

  var errorKey: String? {
    if case let .server(_, body) = self { return body?.error }
    return nil
  }

  var errorCode: Int? {
    if case let .server(_, body) = self { return body?.errorCode }
    return nil
  }

  /// A few words for a watch screen. Server prose is written for larger
  /// screens (it can carry links and next steps), so it is never shown here.
  var displayMessage: String {
    switch self {
    case .signedOut:
      return "Signed out"
    case let .server(status, _):
      switch status {
      case 403: return "Not allowed"
      case 404, 410: return "No longer available"
      case 409: return "Already handled"
      case 429: return "Busy. Try again soon"
      default: return status >= 500 ? "NyxID is unavailable" : "Something went wrong"
      }
    case .transport:
      return "Offline"
    case .decoding:
      return "Update NyxID"
    }
  }
}

/// Seam for tests: the real implementation is `URLSession`.
protocol HTTPTransport: Sendable {
  func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
  /// Starts a streaming request and returns as soon as the response headers
  /// arrive, abandoning a successful body. Error bodies are read (bounded) so
  /// they can be reported.
  func sendDetached(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
}

#if !canImport(FoundationNetworking)
// Apple platforms only; the Linux test host injects its own transport.
struct URLSessionTransport: HTTPTransport {
  let session: URLSession

  init(session: URLSession = URLSessionTransport.makeSession()) {
    self.session = session
  }

  static func makeSession() -> URLSession {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.timeoutIntervalForRequest = 20
    configuration.timeoutIntervalForResource = 40
    configuration.waitsForConnectivity = false
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.urlCache = nil
    return URLSession(configuration: configuration)
  }

  func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
    let (data, response) = try await session.data(for: request)
    guard let http = response as? HTTPURLResponse else {
      throw WatchAPIError.transport("Not an HTTP response")
    }
    return (data, http)
  }

  func sendDetached(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
    let (bytes, response) = try await session.bytes(for: request)
    guard let http = response as? HTTPURLResponse else {
      bytes.task.cancel()
      throw WatchAPIError.transport("Not an HTTP response")
    }
    if (200..<300).contains(http.statusCode) {
      bytes.task.cancel()
      return (Data(), http)
    }
    var data = Data()
    for try await byte in bytes {
      data.append(byte)
      if data.count >= 64 * 1024 { break }
    }
    bytes.task.cancel()
    return (data, http)
  }
}
#endif

/// How the watch identifies itself to NyxID.
enum WatchClientInfo {
  static var appVersion: String {
    (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? "dev"
  }

  /// `X-NyxID-Client-Version` (the server keeps at most 64 characters).
  static var versionHeader: String { String("watchos-\(appVersion)".prefix(64)) }

  static var osVersion: String {
    let version = ProcessInfo.processInfo.operatingSystemVersion
    return "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
  }

  /// Hardware identifier such as `Watch7,1` (`arm64` on the simulator).
  static var hardwareModel: String {
    var info = utsname()
    uname(&info)
    let mirror = Mirror(reflecting: info.machine)
    let bytes = mirror.children.compactMap { $0.value as? Int8 }.prefix { $0 != 0 }
    let value = String(decoding: bytes.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    return value.isEmpty ? "AppleWatch" : value
  }

  /// Fields recorded on the session this watch signs in with, so the person
  /// recognises it in their Sessions list (sanitised again by the server).
  static func loginContext(locale: Locale = .current, timeZone: TimeZone = .current) -> LoginClientContext {
    var fields: [String: String] = [
      "client_label": "Apple Watch",
      "client_app": "NyxID for Apple Watch",
      "client_platform": "watchOS \(osVersion)",
      "client_model": hardwareModel,
      "client_form_factor": "mobile",
      "client_user_agent": "NyxIDWatch/\(appVersion) (watchOS \(osVersion); \(hardwareModel))",
      "client_timezone": timeZone.identifier,
    ]
    let localeId = locale.identifier.replacingOccurrences(of: "_", with: "-")
    if localeId.range(of: #"^[A-Za-z0-9-]{1,35}$"#, options: .regularExpression) != nil {
      fields["client_locale"] = localeId
    }
    return LoginClientContext(fields: fields)
  }
}

enum WatchJSON {
  static let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    decoder.dateDecodingStrategy = .custom { decoder in
      let container = try decoder.singleValueContainer()
      let text = try container.decode(String.self)
      if let date = parseDate(text) { return date }
      throw DecodingError.dataCorruptedError(
        in: container, debugDescription: "Unrecognised date: \(text)")
    }
    return decoder
  }()

  static let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.sortedKeys]
    return encoder
  }()

  /// RFC 3339 with or without fractional seconds. The backend emits `Z` or
  /// `+00:00` offsets and 0-9 fractional digits depending on the endpoint;
  /// fractions are cut to milliseconds before parsing.
  static func parseDate(_ text: String) -> Date? {
    var value = text.trimmingCharacters(in: .whitespaces)
    if let regex = try? NSRegularExpression(pattern: #"\.(\d{1,3})\d*"#) {
      let range = NSRange(value.startIndex..., in: value)
      value = regex.stringByReplacingMatches(in: value, range: range, withTemplate: ".$1")
    }
    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = fractional.date(from: value) { return date }
    let plain = ISO8601DateFormatter()
    plain.formatOptions = [.withInternetDateTime]
    return plain.date(from: value)
  }
}

/// Lets a request body or response be "nothing".
struct EmptyBody: Codable, Equatable {}
