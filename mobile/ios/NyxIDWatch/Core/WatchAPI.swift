import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Authenticated NyxID API client for the watch.
///
/// Owns the watch's session: refreshes the access token shortly before it
/// expires (one refresh at a time, because every refresh rotates the refresh
/// token and replaying a rotated one revokes the whole session), retries a
/// request once after a 401, and signs out when the server refuses the
/// refresh token.
actor WatchAPI {
  private let storage: SessionStorage
  private let transport: HTTPTransport
  private let now: @Sendable () -> Date
  private var session: StoredSession?
  private var refreshing: Task<StoredSession, Error>?
  private var signedOutHandler: (@Sendable () -> Void)?

  /// Refresh this long before the access token expires.
  static let refreshLeeway: TimeInterval = 60

  init(
    storage: SessionStorage,
    transport: HTTPTransport,
    now: @escaping @Sendable () -> Date = { Date() }
  ) {
    self.storage = storage
    self.transport = transport
    self.now = now
    self.session = storage.load()
  }

  #if !canImport(FoundationNetworking)
  init(storage: SessionStorage) {
    self.init(storage: storage, transport: URLSessionTransport())
  }
  #endif

  var isSignedIn: Bool { session != nil }
  var apiBaseURL: URL? { session?.apiBaseURL }

  func onSignedOut(_ handler: @escaping @Sendable () -> Void) {
    signedOutHandler = handler
  }

  // MARK: Session lifecycle

  /// Redeems a one-time login code minted by the iPhone app.
  func redeem(code: String, apiBaseURL: URL, context: LoginClientContext) async throws {
    var body = context.fields
    body["code"] = code
    let request = try Self.makeRequest(
      base: apiBaseURL, method: "POST", path: "/auth/login-code/redeem",
      body: body, accessToken: nil, idempotencyKey: nil)
    let (data, response) = try await send(request)
    guard (200..<300).contains(response.statusCode) else {
      throw Self.serverError(response.statusCode, data)
    }
    let tokens = try Self.decode(RedeemResponse.self, from: data)
    guard tokens.authKind == nil || tokens.authKind == "account_session",
      let accessToken = tokens.accessToken, let refreshToken = tokens.refreshToken
    else {
      // An Agent Key delivery can't drive the watch: it needs an account session.
      throw WatchAPIError.decoding("The code didn't grant an account session.")
    }
    let fresh = StoredSession(
      apiBaseURL: apiBaseURL,
      accessToken: accessToken,
      refreshToken: refreshToken,
      accessExpiresAt: now().addingTimeInterval(TimeInterval(tokens.expiresIn ?? 900)))
    let previous = session
    try storage.save(fresh)
    refreshing?.cancel()
    refreshing = nil
    session = fresh
    // Setting the watch up again (for example after switching accounts on
    // the iPhone) ends the session it replaces instead of orphaning it.
    if let previous, previous.refreshToken != fresh.refreshToken {
      await endOnServer(previous)
    }
  }

  /// Signs out locally and asks the server to end this session.
  func signOut() async {
    let current = session
    clearSession(notify: false)
    guard let current else { return }
    await endOnServer(current)
  }

  /// Ends one watch session and its refresh tokens on the server; the
  /// iPhone's own session is a different one and stays signed in.
  private func endOnServer(_ ended: StoredSession) async {
    if let request = try? Self.makeRequest(
      base: ended.apiBaseURL, method: "POST", path: "/auth/logout",
      body: Optional<EmptyBody>.none,
      accessToken: ended.accessToken, idempotencyKey: nil)
    {
      _ = try? await transport.send(request)
    }
  }

  // MARK: Requests

  func get<T: Decodable>(
    _ path: String, query: [URLQueryItem] = [], as type: T.Type = T.self
  ) async throws -> T {
    try await perform(method: "GET", path: path, query: query, body: Optional<EmptyBody>.none, idempotencyKey: nil)
  }

  func post<Body: Encodable, T: Decodable>(
    _ path: String, body: Body, idempotencyKey: String? = nil, as type: T.Type = T.self
  ) async throws -> T {
    try await perform(method: "POST", path: path, query: [], body: body, idempotencyKey: idempotencyKey)
  }

  /// Starts a server-sent-event request (such as a chat turn) and returns its
  /// response headers without consuming the stream. The server keeps the
  /// work running after the watch disconnects.
  func postDetached<Body: Encodable>(_ path: String, body: Body) async throws -> HTTPURLResponse {
    let (_, response) = try await performRaw(
      method: "POST", path: path, query: [], body: body, idempotencyKey: nil, detached: true)
    return response
  }

  private func perform<Body: Encodable, T: Decodable>(
    method: String, path: String, query: [URLQueryItem], body: Body?, idempotencyKey: String?
  ) async throws -> T {
    let (data, _) = try await performRaw(
      method: method, path: path, query: query, body: body,
      idempotencyKey: idempotencyKey, detached: false)
    if T.self == EmptyBody.self, data.isEmpty { return EmptyBody() as! T }
    return try Self.decode(T.self, from: data)
  }

  private func performRaw<Body: Encodable>(
    method: String, path: String, query: [URLQueryItem], body: Body?,
    idempotencyKey: String?, detached: Bool
  ) async throws -> (Data, HTTPURLResponse) {
    var current = try await validSession()
    for attempt in 0..<2 {
      let request = try Self.makeRequest(
        base: current.apiBaseURL, method: method, path: path, query: query,
        body: body, accessToken: current.accessToken, idempotencyKey: idempotencyKey)
      let (data, response) = try await send(request, detached: detached)
      if response.statusCode == 401, attempt == 0 {
        current = try await refresh(rejected: current)
        continue
      }
      if response.statusCode == 401 {
        clearSession(notify: true)
        throw WatchAPIError.signedOut
      }
      guard (200..<300).contains(response.statusCode) else {
        throw Self.serverError(response.statusCode, data)
      }
      return (data, response)
    }
    throw WatchAPIError.signedOut
  }

  private func validSession() async throws -> StoredSession {
    guard let current = session else { throw WatchAPIError.signedOut }
    if current.accessExpiresAt.timeIntervalSince(now()) > Self.refreshLeeway {
      return current
    }
    return try await refresh(rejected: current)
  }

  /// Refreshes once for every caller holding the same (stale) session.
  private func refresh(rejected: StoredSession) async throws -> StoredSession {
    if let current = session, current.accessToken != rejected.accessToken {
      return current
    }
    if let refreshing {
      return try await refreshing.value
    }
    guard let current = session else { throw WatchAPIError.signedOut }
    let task = Task { try await self.performRefresh(current) }
    refreshing = task
    defer { refreshing = nil }
    return try await task.value
  }

  private func performRefresh(_ current: StoredSession) async throws -> StoredSession {
    let request = try Self.makeRequest(
      base: current.apiBaseURL, method: "POST", path: "/auth/refresh",
      body: ["refresh_token": current.refreshToken], accessToken: nil, idempotencyKey: nil)
    var (data, response) = try await send(request)
    if response.statusCode == 409 {
      // "Refresh token was concurrently rotated": the server resolves the
      // race within its grace window, so one short retry is enough.
      try? await Task.sleep(nanoseconds: 200_000_000)
      (data, response) = try await send(request)
    }
    if response.statusCode == 401 || response.statusCode == 403 {
      // The session was revoked (signed out elsewhere, or an old token replayed).
      clearSession(notify: true)
      throw WatchAPIError.signedOut
    }
    guard (200..<300).contains(response.statusCode) else {
      throw Self.serverError(response.statusCode, data)
    }
    let tokens = try Self.decode(RefreshResponse.self, from: data)
    let next = StoredSession(
      apiBaseURL: current.apiBaseURL,
      accessToken: tokens.accessToken,
      refreshToken: tokens.refreshToken ?? current.refreshToken,
      accessExpiresAt: now().addingTimeInterval(TimeInterval(tokens.expiresIn ?? 900)))
    try storage.save(next)
    // A sign-out that happened during the refresh wins.
    guard session?.refreshToken == current.refreshToken else { throw WatchAPIError.signedOut }
    session = next
    return next
  }

  private func clearSession(notify: Bool) {
    refreshing?.cancel()
    refreshing = nil
    let hadSession = session != nil
    session = nil
    storage.clear()
    if notify, hadSession { signedOutHandler?() }
  }

  private func send(_ request: URLRequest, detached: Bool = false) async throws -> (Data, HTTPURLResponse) {
    do {
      return detached ? try await transport.sendDetached(request) : try await transport.send(request)
    } catch let error as WatchAPIError {
      throw error
    } catch {
      throw WatchAPIError.transport(String(describing: type(of: error)))
    }
  }

  // MARK: Helpers

  static func makeRequest<Body: Encodable>(
    base: URL, method: String, path: String, query: [URLQueryItem] = [],
    body: Body?, accessToken: String?, idempotencyKey: String?
  ) throws -> URLRequest {
    guard var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
      throw WatchAPIError.transport("Bad base URL")
    }
    components.path = components.path + path
    if !query.isEmpty { components.queryItems = query }
    guard let url = components.url else { throw WatchAPIError.transport("Bad path") }
    var request = URLRequest(url: url)
    request.httpMethod = method
    request.setValue("application/json", forHTTPHeaderField: "Accept")
    // Telemetry and approval `decided_via` attribution; the backend only
    // knows ui/cli/mobile/sdk, and the watch is part of the mobile client.
    request.setValue("mobile", forHTTPHeaderField: "X-NyxID-Client")
    request.setValue(WatchClientInfo.versionHeader, forHTTPHeaderField: "X-NyxID-Client-Version")
    if let accessToken {
      request.setValue("Bearer \(accessToken)", forHTTPHeaderField: "Authorization")
    }
    if let idempotencyKey {
      request.setValue(idempotencyKey, forHTTPHeaderField: "Idempotency-Key")
    }
    if let body {
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.httpBody = try WatchJSON.encoder.encode(body)
    }
    return request
  }

  static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
    do {
      return try WatchJSON.decoder.decode(T.self, from: data)
    } catch {
      throw WatchAPIError.decoding(String(describing: error).prefix(300).description)
    }
  }

  static func serverError(_ status: Int, _ data: Data) -> WatchAPIError {
    .server(status: status, body: try? WatchJSON.decoder.decode(APIErrorBody.self, from: data))
  }
}

/// Attribution the server records for the sign-in this watch performs
/// (shown to the person in their Sessions list).
struct LoginClientContext: Equatable, Sendable {
  var fields: [String: String]
}

/// `POST /auth/login-code/redeem`: tagged by `auth_kind`. Only an account
/// session can drive the watch (agent chat routes refuse Agent Keys).
struct RedeemResponse: Decodable {
  let authKind: String?
  let accessToken: String?
  let refreshToken: String?
  let expiresIn: Int?
}

/// `POST /auth/refresh`: the refresh token rotates on every call.
struct RefreshResponse: Decodable {
  let accessToken: String
  let refreshToken: String?
  let expiresIn: Int?
}
