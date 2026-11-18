import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

@testable import NyxIDWatchCore

/// Scripted HTTP server for WatchAPI tests.
final class FakeTransport: HTTPTransport, @unchecked Sendable {
  struct Reply {
    let status: Int
    let body: String
    var headers: [String: String] = [:]
  }

  typealias Handler = (URLRequest) -> Reply

  private let lock = NSLock()
  private var handlers: [(method: String, path: String, handler: Handler)] = []
  private(set) var requests: [URLRequest] = []
  var delay: UInt64 = 0

  func on(_ method: String, _ path: String, _ handler: @escaping Handler) {
    lock.lock()
    handlers.append((method, path, handler))
    lock.unlock()
  }

  func on(_ method: String, _ path: String, status: Int = 200, json: String, headers: [String: String] = [:]) {
    on(method, path) { _ in Reply(status: status, body: json, headers: headers) }
  }

  func calls(_ method: String, _ path: String) -> [URLRequest] {
    lock.lock()
    defer { lock.unlock() }
    return requests.filter { $0.httpMethod == method && $0.url?.path == path }
  }

  func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
    if delay > 0 { try await Task.sleep(nanoseconds: delay) }
    let path = request.url?.path ?? ""
    let match = lock.withLock {
      requests.append(request)
      return handlers.last { $0.method == request.httpMethod && $0.path == path }
    }
    let reply = match?.handler(request) ?? Reply(status: 404, body: #"{"error":"not_found","error_code":1003,"message":"Not found"}"#)
    let response = HTTPURLResponse(
      url: request.url!, statusCode: reply.status, httpVersion: "HTTP/1.1",
      headerFields: reply.headers.merging(["Content-Type": "application/json"]) { a, _ in a })!
    return (Data(reply.body.utf8), response)
  }

  func sendDetached(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
    let (data, response) = try await send(request)
    return ((200..<300).contains(response.statusCode) ? Data() : data, response)
  }
}

extension URLRequest {
  var jsonBody: [String: Any] {
    guard let httpBody, let object = try? JSONSerialization.jsonObject(with: httpBody) as? [String: Any] else {
      return [:]
    }
    return object
  }
}

enum Fixtures {
  static let base = URL(string: "https://auth.example.com/api/v1")!

  static func session(expiresIn seconds: TimeInterval = 600, now: Date = Date()) -> StoredSession {
    StoredSession(
      apiBaseURL: base, accessToken: "access-1", refreshToken: "refresh-1",
      accessExpiresAt: now.addingTimeInterval(seconds))
  }

  static func approval(
    id: String = "11111111-1111-4111-8111-111111111111", mode: String = "per_request",
    proposal: Bool = false, status: String = "pending", createdAt: String = "2026-11-16T10:00:00.123456+00:00"
  ) -> String {
    """
    {"id":"\(id)","service_name":"Gmail","service_slug":"gmail","requester_type":"agent",
     "requester_label":"Claude Code","operation_summary":"proxy:POST /send","action_description":null,
     "http_method":"POST","resource":"/send","verb":"write","approval_mode":"\(mode)","status":"\(status)",
     "created_at":"\(createdAt)","expires_at":"2026-11-16T10:05:00+00:00","decided_at":null,
     "decision_channel":null,"from_org_policy":false
     \(proposal ? #","execution_proposal":{"version":2,"state":"pending","review_path":"/assistant/approvals","supported":true,"can_decide":true,"revision":1,"choices":[]}"# : "")}
    """
  }
}
