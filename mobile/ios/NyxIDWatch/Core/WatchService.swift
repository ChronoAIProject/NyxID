import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// The NyxID operations the watch performs, on top of `WatchAPI`.
struct WatchService: Sendable {
  let api: WatchAPI

  // MARK: Approvals

  func pendingApprovals() async throws -> [ApprovalRequest] {
    let page: ApprovalPage = try await api.get(
      "/approvals/requests",
      query: [
        URLQueryItem(name: "status", value: "pending"),
        URLQueryItem(name: "page", value: "1"),
        URLQueryItem(name: "per_page", value: "50"),
        URLQueryItem(name: "include_admin_orgs", value: "true"),
      ])
    return page.requests
      .filter { $0.status == "pending" }
      .sorted { $0.createdAt > $1.createdAt }
  }

  func approval(id: String) async throws -> ApprovalRequest {
    try await api.get("/approvals/requests/\(try PathSegment(id))")
  }

  /// Approves or denies like the iPhone app does: a grant-mode approval
  /// lasts the person's configured grant period (30 days by default).
  func decide(
    _ request: ApprovalRequest, approve: Bool, idempotencyKey: String
  ) async throws -> ApprovalDecisionResult {
    guard request.executionProposal == nil else { throw WatchServiceError.reviewOnPhone }
    var duration: Int?
    if approve, request.isGrant {
      let settings: NotificationSettings? = try? await api.get("/notifications/settings")
      duration = max(1, settings?.grantExpiryDays ?? 30) * 86_400
    }
    return try await api.post(
      "/approvals/requests/\(try PathSegment(request.id))/decide",
      body: ApprovalDecision(approved: approve, durationSec: duration),
      idempotencyKey: idempotencyKey)
  }

  /// How many days a grant-mode approval lasts (the person's setting).
  func grantExpiryDays() async -> Int? {
    let settings: NotificationSettings? = try? await api.get("/notifications/settings")
    return settings?.grantExpiryDays
  }

  static func decisionKey(for requestId: String, now: Date = Date()) -> String {
    let millis = Int(now.timeIntervalSince1970 * 1000)
    let random = UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(8).lowercased()
    return "decision:\(requestId):\(millis):\(random)"
  }

  // MARK: NyxBot notifications feed

  /// `nil` when the feed isn't enabled for this account.
  func feedSummary() async throws -> FeedSummary? {
    do {
      let summary: FeedSummary = try await api.get("/assistant/nyxagent/notifications/summary")
      return summary
    } catch let error as WatchAPIError where error.isNotFound {
      return nil
    }
  }

  /// `nil` when the feed isn't enabled for this account.
  func feed(cursor: String? = nil, limit: Int = 20, unreadOnly: Bool = false) async throws -> FeedPage? {
    var query = [URLQueryItem(name: "limit", value: String(limit))]
    if unreadOnly { query.append(URLQueryItem(name: "unread_only", value: "true")) }
    if let cursor { query.append(URLQueryItem(name: "cursor", value: cursor)) }
    do {
      let page: FeedPage = try await api.get("/assistant/nyxagent/notifications", query: query)
      return page
    } catch let error as WatchAPIError where error.isNotFound {
      return nil
    }
  }

  func markRead(_ ids: [String]) async throws {
    guard !ids.isEmpty else { return }
    let _: MarkReadResult = try await api.post(
      "/assistant/nyxagent/notifications/read", body: MarkReadByIds(ids: Array(ids.prefix(100))))
  }

  /// Marks everything from one thread as seen (opening it reads it).
  func markThreadRead(_ conversationId: String) async throws {
    let _: MarkReadResult = try await api.post(
      "/assistant/nyxagent/notifications/read", body: MarkReadBySource.conversation(conversationId))
  }

  // MARK: Account

  func profile() async throws -> UserProfile {
    try await api.get("/users/me")
  }

  // MARK: Agents and chat

  /// Stops the agent's current reply in a thread.
  func stop(conversationId: String) async throws {
    let _: EmptyBody = try await api.post(
      "/assistant/nyxagent/conversations/\(try PathSegment(conversationId))/stop", body: EmptyBody())
  }

  /// Agents the person can talk to, NyxBot first.
  func agents() async throws -> [AgentSummary] {
    let list: AgentList = try await api.get("/assistant/nyxagent/agents")
    let usable = list.agents.filter(\.canChat)
    return usable.filter(\.isNyxBot) + usable.filter { !$0.isNyxBot }
  }

  /// The thread the watch continues for an agent: its home thread, else its
  /// most recent one, else none (the first message starts a new thread).
  func threadId(agentId: String, home: String?) async throws -> String? {
    if let home { return home }
    let list: ConversationList = try await api.get(
      "/assistant/nyxagent/conversations",
      query: [
        URLQueryItem(name: "agent_id", value: agentId),
        URLQueryItem(name: "limit", value: "1"),
      ])
    return list.conversations.first?.id
  }

  /// An agent's threads, newest first, without untouched empty ones.
  func threads(agentId: String, limit: Int = 30) async throws -> [ConversationSummary] {
    let list: ConversationList = try await api.get(
      "/assistant/nyxagent/conversations",
      query: [
        URLQueryItem(name: "agent_id", value: agentId),
        URLQueryItem(name: "limit", value: String(limit)),
      ])
    return list.conversations
      .filter { !$0.isEmpty }
      .sorted { ($0.lastMessageAt ?? .distantPast) > ($1.lastMessageAt ?? .distantPast) }
  }

  /// The newest `limit` messages of a thread, or those before `beforeSeq`.
  func history(conversationId: String, limit: Int = 30, beforeSeq: Int? = nil) async throws
    -> ConversationHistory
  {
    var query = [URLQueryItem(name: "limit", value: String(limit))]
    if let beforeSeq { query.append(URLQueryItem(name: "before_seq", value: String(beforeSeq))) }
    return try await api.get(
      "/assistant/nyxagent/conversations/\(try PathSegment(conversationId))", query: query)
  }

  /// Sends a message and returns the thread it went to. The reply keeps
  /// running on the server; follow it with `history`.
  func send(_ turn: TurnRequest) async throws -> String {
    let response = try await api.postDetached("/assistant/nyxagent/turns", body: turn)
    if let id = response.value(forHTTPHeaderField: "x-nyxid-conversation-id"), !id.isEmpty {
      return id
    }
    if let id = turn.conversationId { return id }
    throw WatchAPIError.decoding("The reply didn't name its conversation.")
  }
}

enum WatchServiceError: Error, Equatable {
  case reviewOnPhone
}

/// A server identifier used as one URL path segment.
struct PathSegment: CustomStringConvertible {
  let value: String

  init(_ value: String) throws {
    guard value.range(of: #"^[A-Za-z0-9_-]{1,128}$"#, options: .regularExpression) != nil else {
      throw WatchAPIError.decoding("Unexpected identifier")
    }
    self.value = value
  }

  var description: String { value }
}

extension WatchAPIError {
  var isNotFound: Bool {
    if case let .server(status, _) = self { return status == 404 }
    return false
  }
}

/// Where a sent message stands, judged from the thread's history.
enum TurnProgress: Equatable {
  /// No sign of the turn yet (the next poll should show it).
  case pending
  /// The agent is working; `partial` is the reply streamed so far.
  case working(partial: String?, queued: Bool)
  case replied(ChatMessage)
  case failed(String)

  static func evaluate(_ history: ConversationHistory, afterSeq baseline: Int) -> TurnProgress {
    if let turn = history.conversation.activeTurn {
      let partial = turn.reply?.text.flatMap { $0.isEmpty ? nil : $0 }
      return .working(partial: partial, queued: (turn.queued ?? false) || (turn.waitingForCapacity ?? false))
    }
    let fresh = history.messages.filter { $0.seq > baseline }
    if let reply = fresh.last(where: \.isReply) {
      return reply.failed ? .failed(failureText(reply)) : .replied(reply)
    }
    if fresh.contains(where: \.isFromPerson) {
      return .failed("The reply was interrupted. Try again.")
    }
    return .pending
  }

  static func failureText(_ message: ChatMessage) -> String {
    switch message.errorCode {
    case "cancelled": return "Stopped."
    case "queue_timeout": return "NyxID was busy. Try again in a moment."
    case "turn_lost", "outcome_unknown": return "The reply was interrupted. Try again."
    default:
      if let detail = message.errorDetail?.message, !detail.isEmpty { return detail }
      if !message.text.isEmpty { return SpokenText.plain(message.text) }
      return "The agent couldn't answer. Try again."
    }
  }
}
