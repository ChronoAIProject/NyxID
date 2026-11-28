import Foundation
import XCTest

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

@testable import NyxIDWatchCore

final class WatchServiceTests: XCTestCase {
  private func service(_ transport: FakeTransport) -> WatchService {
    WatchService(api: WatchAPI(storage: MemorySessionStorage(Fixtures.session()), transport: transport))
  }

  func testPendingApprovalsAreNewestFirstAndProposalsAreReviewOnly() async throws {
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/approvals/requests") { request in
      let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
      XCTAssertTrue(query.contains(URLQueryItem(name: "status", value: "pending")))
      XCTAssertTrue(query.contains(URLQueryItem(name: "include_admin_orgs", value: "true")))
      let older = Fixtures.approval(id: "a1", createdAt: "2026-11-16T09:00:00+00:00")
      let newer = Fixtures.approval(id: "a2", proposal: true, createdAt: "2026-11-16T11:00:00.5+00:00")
      return .init(status: 200, body: #"{"requests":[\#(older),\#(newer)],"total":2,"page":1,"per_page":50}"#)
    }

    let approvals = try await service(transport).pendingApprovals()

    XCTAssertEqual(approvals.map(\.id), ["a2", "a1"])
    XCTAssertFalse(approvals[0].canDecideOnWatch)
    XCTAssertTrue(approvals[1].canDecideOnWatch)
    XCTAssertEqual(approvals[1].risk, .medium)
    XCTAssertEqual(approvals[1].requester, "Claude Code")
  }

  func testApproveSendsTheIdempotencyKeyAndNoDurationForPerRequest() async throws {
    let transport = FakeTransport()
    let id = "11111111-1111-4111-8111-111111111111"
    transport.on("POST", "/api/v1/approvals/requests/\(id)/decide") { request in
      XCTAssertEqual(request.value(forHTTPHeaderField: "Idempotency-Key"), "decision:key")
      XCTAssertEqual(request.jsonBody["approved"] as? Bool, true)
      XCTAssertNil(request.jsonBody["duration_sec"])
      return .init(status: 200, body: #"{"id":"\#(id)","status":"approved","decided_at":"2026-11-16T10:01:00+00:00"}"#)
    }
    let approval = try WatchJSON.decoder.decode(ApprovalRequest.self, from: Data(Fixtures.approval(id: id).utf8))

    let result = try await service(transport).decide(approval, approve: true, idempotencyKey: "decision:key")

    XCTAssertEqual(result.status, "approved")
    XCTAssertTrue(transport.calls("GET", "/api/v1/notifications/settings").isEmpty)
  }

  func testGrantApprovalUsesTheConfiguredGrantPeriod() async throws {
    let transport = FakeTransport()
    let id = "22222222-2222-4222-8222-222222222222"
    transport.on("GET", "/api/v1/notifications/settings", json: #"{"id":"s","grant_expiry_days":7,"push_enabled":true}"#)
    transport.on("POST", "/api/v1/approvals/requests/\(id)/decide") { request in
      XCTAssertEqual(request.jsonBody["duration_sec"] as? Int, 7 * 86_400)
      return .init(status: 200, body: #"{"id":"\#(id)","status":"approved"}"#)
    }
    let approval = try WatchJSON.decoder.decode(
      ApprovalRequest.self, from: Data(Fixtures.approval(id: id, mode: "grant").utf8))

    _ = try await service(transport).decide(approval, approve: true, idempotencyKey: "k")
  }

  func testDenyNeverSendsADuration() async throws {
    let transport = FakeTransport()
    let id = "33333333-3333-4333-8333-333333333333"
    transport.on("POST", "/api/v1/approvals/requests/\(id)/decide") { request in
      XCTAssertEqual(request.jsonBody["approved"] as? Bool, false)
      XCTAssertNil(request.jsonBody["duration_sec"])
      return .init(status: 200, body: #"{"id":"\#(id)","status":"rejected"}"#)
    }
    let approval = try WatchJSON.decoder.decode(
      ApprovalRequest.self, from: Data(Fixtures.approval(id: id, mode: "grant").utf8))

    _ = try await service(transport).decide(approval, approve: false, idempotencyKey: "k")
  }

  func testProposalsAreNeverDecidedFromTheWatch() async throws {
    let transport = FakeTransport()
    let approval = try WatchJSON.decoder.decode(
      ApprovalRequest.self, from: Data(Fixtures.approval(proposal: true).utf8))
    do {
      _ = try await service(transport).decide(approval, approve: true, idempotencyKey: "k")
      XCTFail("expected a refusal")
    } catch let error as WatchServiceError {
      XCTAssertEqual(error, .reviewOnPhone)
    }
    XCTAssertTrue(transport.requests.isEmpty)
  }

  func testOwnerChatQueueAndDetailNeverSendADecision() async throws {
    let transport = FakeTransport()
    let id = "owner-chat"
    let wire = Fixtures.approval(id: id, mode: "grant")
      .replacingOccurrences(of: #""decision_channel":null"#,
        with: #""decision_channel":null,"decision_destination":"originating_owner_chat""#)
      .replacingOccurrences(of: "2026-11-16T10:05:00+00:00", with: "2099-01-01T00:00:00+00:00")
    transport.on("GET", "/api/v1/approvals/requests", json: "{\"requests\":[\(wire)],\"total\":1}")
    transport.on("GET", "/api/v1/approvals/requests/\(id)", json: wire)
    let queued = try await service(transport).pendingApprovals()
    let detail = try await service(transport).approval(id: id)
    XCTAssertEqual(queued.count, 1)
    for request in [queued[0], detail] {
      XCTAssertFalse(request.canDecideOnWatch)
      XCTAssertEqual(request.reviewCue, "Review in original NyxBot chat")
      for allow in [false, true] {
        do {
          _ = try await service(transport).decide(request, approve: allow, idempotencyKey: "no-decision")
          XCTFail("original owner chat is required")
        } catch let error as WatchServiceError {
          XCTAssertEqual(error, .reviewInOwnerChat)
        }
      }
    }
    XCTAssertEqual(transport.requests.count, 2)
    XCTAssertTrue(transport.calls("POST", "/api/v1/approvals/requests/\(id)/decide").isEmpty)
    XCTAssertTrue(transport.calls("GET", "/api/v1/notifications/settings").isEmpty)
  }

  func testFeedIsOptional() async throws {
    let transport = FakeTransport()
    transport.on(
      "GET", "/api/v1/assistant/nyxagent/notifications/summary", status: 404,
      json: #"{"error":"not_found","error_code":1003,"message":"Not found: Assistant route not found."}"#)
    let summary = try await service(transport).feedSummary()
    XCTAssertNil(summary)
  }

  func testFeedItemsDecode() async throws {
    let transport = FakeTransport()
    transport.on(
      "GET", "/api/v1/assistant/nyxagent/notifications",
      json: """
        {"items":[{"id":"n1","kind":"automation_result","count":1,
          "source":{"type":"trigger","id":"t1","label":"Daily brief","conversation_id":null,"group_id":null,"channel_id":null,"trigger_id":"t1","agent_id":null},
          "actor":{"type":"automation","id":"t1","name":"Daily brief","display_name":null,"kind":null},
          "platform":null,"participants":[],"outcome":"succeeded","reason":null,"title":"Daily brief",
          "preview":"3 meetings today","url":"https://app.example/assistant","read":false,"resolved":false,
          "created_at":"2026-11-10T08:00:00.000Z","updated_at":"2026-11-10T08:05:00.000Z"}],
         "next_cursor":null,"as_of":"2026-11-10T08:06:00.000Z"}
        """)
    let page = try await service(transport).feed()
    XCTAssertEqual(page?.items.first?.heading, "Daily brief")
    XCTAssertEqual(page?.items.first?.preview, "3 meetings today")
  }

  func testAgentsPutNyxBotFirstAndHideUnusableOnes() async throws {
    let transport = FakeTransport()
    transport.on(
      "GET", "/api/v1/assistant/nyxagent/agents",
      json: """
        {"agents":[
          {"id":"s1","kind":"specialist","name":"research","display_name":"Researcher","status":"idle","can_use":true,"home_conversation_id":"nyxa-1"},
          {"id":"s2","kind":"specialist","name":"gone","display_name":null,"status":"destroyed","can_use":true},
          {"id":"s3","kind":"specialist","name":"shared","display_name":"Shared agent · access ended","status":"idle","can_use":false},
          {"id":"b1","kind":"nyxbot","name":"nyxbot","display_name":null,"status":"running","can_use":true,"home_conversation_id":null}
        ],"destroyed_truncated":false}
        """)
    let agents = try await service(transport).agents()
    XCTAssertEqual(agents.map(\.id), ["b1", "s1"])
    XCTAssertEqual(agents.map(\.label), ["NyxBot", "Researcher"])
  }

  func testAgentThreadsAreNewestFirstWithoutEmptyOnes() async throws {
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/assistant/nyxagent/conversations") { request in
      let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
      XCTAssertTrue(query.contains(URLQueryItem(name: "agent_id", value: "s1")))
      return .init(
        status: 200,
        body: """
          {"conversations":[
            {"id":"nyxa-old","title":"Old trip","last_message_at":"2026-11-10T10:00:00Z","message_count":4},
            {"id":"nyxa-home","title":"","last_message_at":"2026-11-17T10:00:00Z","message_count":0},
            {"id":"nyxa-new","title":"  ","last_message_at":"2026-11-16T10:00:00Z","message_count":2}
          ],"next_cursor":null}
          """)
    }
    let threads = try await service(transport).threads(agentId: "s1")
    XCTAssertEqual(threads.map(\.id), ["nyxa-new", "nyxa-old"])
    XCTAssertEqual(threads.map(\.label), ["New chat", "Old trip"])
  }

  func testUnreadFeedAsksForUnreadOnly() async throws {
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/assistant/nyxagent/notifications") { request in
      let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
      XCTAssertTrue(query.contains(URLQueryItem(name: "unread_only", value: "true")))
      return .init(status: 200, body: #"{"items":[],"next_cursor":null}"#)
    }
    let page = try await service(transport).feed(limit: 50, unreadOnly: true)
    XCTAssertEqual(page?.items.count, 0)
  }

  func testOpeningAThreadMarksItsSourceRead() async throws {
    let transport = FakeTransport()
    transport.on("POST", "/api/v1/assistant/nyxagent/notifications/read") { request in
      let source = request.jsonBody["source"] as? [String: Any]
      XCTAssertEqual(source?["type"] as? String, "conversation")
      XCTAssertEqual(source?["id"] as? String, "nyxa-1")
      XCTAssertNil(request.jsonBody["ids"])
      return .init(status: 200, body: #"{"updated":2}"#)
    }
    try await service(transport).markThreadRead("nyxa-1")
    XCTAssertEqual(transport.calls("POST", "/api/v1/assistant/nyxagent/notifications/read").count, 1)
  }

  func testEarlierMessagesUseTheHistoryCursor() async throws {
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/assistant/nyxagent/conversations/nyxa-1") { request in
      let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
      XCTAssertTrue(query.contains(URLQueryItem(name: "before_seq", value: "41")))
      return .init(
        status: 200,
        body: #"{"conversation":{"id":"nyxa-1","title":"t","active_turn":null},"messages":[],"before_seq":11}"#)
    }
    let page = try await service(transport).history(conversationId: "nyxa-1", beforeSeq: 41)
    XCTAssertEqual(page.beforeSeq, 11)
  }

  func testAThreadIsFoundForAnAgentWithoutAHome() async throws {
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/assistant/nyxagent/conversations") { request in
      let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
      XCTAssertTrue(query.contains(URLQueryItem(name: "agent_id", value: "s1")))
      return .init(status: 200, body: #"{"conversations":[{"id":"nyxa-9","title":"x"}]}"#)
    }
    let found = try await service(transport).threadId(agentId: "s1", home: nil)
    XCTAssertEqual(found, "nyxa-9")
    let home = try await service(transport).threadId(agentId: "s1", home: "nyxa-home")
    XCTAssertEqual(home, "nyxa-home")
  }

  func testSendReturnsTheThreadFromTheResponseHeader() async throws {
    let transport = FakeTransport()
    transport.on("POST", "/api/v1/assistant/nyxagent/turns") { request in
      XCTAssertEqual(request.jsonBody["agent_id"] as? String, "b1")
      XCTAssertEqual(request.jsonBody["text"] as? String, "What's on today?")
      XCTAssertNil(request.jsonBody["conversation_id"])
      return .init(
        status: 200, body: "", headers: ["x-nyxid-conversation-id": "nyxa-0123456789abcdef0123456789abcdef"])
    }
    let id = try await service(transport).send(.newThread(with: "b1", text: "What's on today?"))
    XCTAssertEqual(id, "nyxa-0123456789abcdef0123456789abcdef")
  }

  func testBusyThreadSurfacesTurnActive() async throws {
    let transport = FakeTransport()
    transport.on(
      "POST", "/api/v1/assistant/nyxagent/turns", status: 409,
      json: #"{"error":"turn_active","error_code":12100,"message":"A turn is already active in this conversation"}"#)
    do {
      _ = try await service(transport).send(.reply(in: "nyxa-1", text: "hi"))
      XCTFail("expected turn_active")
    } catch let error as WatchAPIError {
      XCTAssertEqual(error.errorKey, "turn_active")
    }
  }

  func testRequesterWordingFallsBackToAReadableKind() throws {
    let unnamed = Fixtures.approval().replacingOccurrences(
      of: #""requester_label":"Claude Code""#, with: #""requester_label":null"#)
    let approval = try WatchJSON.decoder.decode(ApprovalRequest.self, from: Data(unnamed.utf8))
    XCTAssertEqual(approval.requester, "agent")
    let keyed = unnamed.replacingOccurrences(of: #""requester_type":"agent""#, with: #""requester_type":"api_key""#)
    XCTAssertEqual(
      try WatchJSON.decoder.decode(ApprovalRequest.self, from: Data(keyed.utf8)).requester, "an agent key")
  }

  func testPathSegmentsRejectTraversal() {
    XCTAssertThrowsError(try PathSegment("../users"))
    XCTAssertThrowsError(try PathSegment(""))
    XCTAssertNoThrow(try PathSegment("nyxa-0123456789abcdef0123456789abcdef"))
  }
}

final class TurnProgressTests: XCTestCase {
  private func history(active: String? = nil, messages: [String]) throws -> ConversationHistory {
    let turn = active.map { #"{"turn_id":"t2","reply":{"text":"\#($0)"},"queued":false,"waiting_for_capacity":false}"# } ?? "null"
    let json = """
      {"conversation":{"id":"nyxa-1","title":"x","active_turn":\(turn),"pending_acknowledgements":0},
       "messages":[\(messages.joined(separator: ","))],"acknowledgements":[],"approvals":[]}
      """
    return try WatchJSON.decoder.decode(ConversationHistory.self, from: Data(json.utf8))
  }

  private func message(seq: Int, role: String, text: String, status: String = "completed", error: String? = nil) -> String {
    let code = error.map { "\"\($0)\"" } ?? "null"
    return """
      {"id":"m\(seq)","seq":\(seq),"turn_id":"t\(seq)","role":"\(role)","text":"\(text)","status":"\(status)",
       "error_code":\(code),"error_detail":null,"created_at":"2026-11-16T10:00:0\(seq)Z"}
      """
  }

  func testWorkingWhileTheTurnIsActive() throws {
    let progress = TurnProgress.evaluate(
      try history(active: "Checking your cal", messages: [message(seq: 3, role: "user", text: "hi")]), afterSeq: 2)
    XCTAssertEqual(progress, .working(partial: "Checking your cal", queued: false))
  }

  func testRepliedOnceTheAssistantMessageLands() throws {
    let progress = TurnProgress.evaluate(
      try history(messages: [
        message(seq: 1, role: "user", text: "old"), message(seq: 2, role: "assistant", text: "old reply"),
        message(seq: 3, role: "user", text: "hi"), message(seq: 4, role: "assistant", text: "Hello!"),
      ]),
      afterSeq: 2)
    guard case let .replied(reply) = progress else { return XCTFail("expected a reply, got \(progress)") }
    XCTAssertEqual(reply.text, "Hello!")
  }

  func testFailedReplies() throws {
    let progress = TurnProgress.evaluate(
      try history(messages: [
        message(seq: 3, role: "user", text: "hi"),
        message(seq: 4, role: "assistant", text: "", status: "failed", error: "queue_timeout"),
      ]),
      afterSeq: 2)
    XCTAssertEqual(progress, .failed("NyxID was busy. Try again in a moment."))
  }

  func testOldRepliesDontCountAsTheAnswer() throws {
    let progress = TurnProgress.evaluate(
      try history(messages: [message(seq: 2, role: "assistant", text: "old reply")]), afterSeq: 2)
    XCTAssertEqual(progress, .pending)
  }
}
