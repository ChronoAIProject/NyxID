import Foundation
import XCTest

@testable import NyxIDWatchCore

final class RelativeTimeTests: XCTestCase {
  private var calendar: Calendar {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return calendar
  }

  func testShortAges() {
    let now = WatchJSON.parseDate("2026-11-18T15:00:00Z")!
    func ago(_ seconds: TimeInterval) -> String {
      RelativeTime.short(now.addingTimeInterval(-seconds), now: now, calendar: calendar)
    }
    XCTAssertEqual(ago(10), "now")
    XCTAssertEqual(ago(44 * 60), "44m")
    XCTAssertEqual(ago(3 * 3_600), "3h")
    XCTAssertEqual(ago(20 * 3_600), "Yesterday")
    XCTAssertEqual(ago(3 * 86_400), "Sun")
    XCTAssertEqual(ago(15 * 86_400), "3 Nov")
  }
}

final class ToolActivityTextTests: XCTestCase {
  // Expected values match mobile/src/features/assistant/assistantActivityText.ts.
  func testToolCallsReadAsActions() {
    XCTAssertEqual(ToolActivityText.running("api-google-calendar__list_events"), "Listing events on Google Calendar")
    XCTAssertEqual(ToolActivityText.running("api-github-4__get_issue"), "Reading issue on GitHub")
    XCTAssertEqual(ToolActivityText.running("nyx__search_tools"), "Searching for tools")
    XCTAssertEqual(ToolActivityText.running("nyxid__remember"), "Remembering")
    XCTAssertEqual(ToolActivityText.running("api-notion__pages"), "Using pages on Notion")
    // TypeScript shows an unknown identifier as-is; a watch says "Working".
    XCTAssertEqual(ToolActivityText.running("mystery"), "Working")
  }

  func testTheTurnStatusLine() throws {
    let json = """
      {"turn_id":"t","reply":null,"queued":false,"waiting_for_capacity":false,
       "activities":[{"id":"a1","label":"nyx__search_tools","status":"completed"},
                     {"id":"a2","label":"api-google-calendar__list_events","status":"running"}]}
      """
    let turn = try WatchJSON.decoder.decode(ActiveTurn.self, from: Data(json.utf8))
    XCTAssertEqual(turn.statusLine, "Listing events on Google Calendar…")
    let idle = try WatchJSON.decoder.decode(ActiveTurn.self, from: Data(#"{"turn_id":"t"}"#.utf8))
    XCTAssertEqual(idle.statusLine, "Thinking…")
  }
}

final class ApprovalPresentationTests: XCTestCase {
  private func approval(_ json: String) throws -> ApprovalRequest {
    try WatchJSON.decoder.decode(ApprovalRequest.self, from: Data(json.utf8))
  }

  func testOwnerChatCuePreservesExpirationAndTerminalState() throws {
    for (status, expiry, cue) in [
      ("pending", "2099-01-01T00:00:00+00:00", "Review in original NyxBot chat"),
      ("pending", "2000-01-01T00:00:00+00:00", "Expired"),
      ("expired", "2099-01-01T00:00:00+00:00", "Expired"),
      ("approved", "2000-01-01T00:00:00+00:00", "Approved"),
      ("rejected", "2000-01-01T00:00:00+00:00", "Denied"),
    ] {
      let request = try approval(Fixtures.approval(status: status)
        .replacingOccurrences(of: #""decision_channel":null"#,
          with: #""decision_channel":null,"decision_destination":"originating_owner_chat""#)
        .replacingOccurrences(of: "2026-11-16T10:05:00+00:00", with: expiry))
      XCTAssertFalse(request.canDecideOnWatch)
      XCTAssertEqual(request.reviewCue, cue)
    }
    XCTAssertTrue(try approval(Fixtures.approval()).canDecideOnWatch)
    XCTAssertEqual(try approval(Fixtures.approval(proposal: true)).reviewCue, "Review on iPhone")
  }

  func testCardWording() throws {
    let write = try approval(Fixtures.approval(mode: "grant"))
    XCTAssertEqual(write.headline, "Make changes in Gmail")
    XCTAssertEqual(write.askedBy, "Claude Code wants access")
    XCTAssertEqual(write.accessLabel, "Makes changes")
    XCTAssertEqual(write.durationLabel(grantDays: 7), "7 days")
    XCTAssertEqual(write.durationLabel(grantDays: 1), "1 day")

    let described = try approval(
      Fixtures.approval().replacingOccurrences(
        of: #""action_description":null"#, with: #""action_description":"read your inbox""#))
    XCTAssertEqual(described.headline, "Read your inbox")
    XCTAssertEqual(described.durationLabel(grantDays: 30), "Once")
  }

  func testStackSummaryNamesUpToTwoServices() throws {
    let gmail = try approval(Fixtures.approval(id: "a1"))
    let calendar = try approval(Fixtures.approval(id: "a2").replacingOccurrences(of: "Gmail", with: "Calendar"))
    let drive = try approval(Fixtures.approval(id: "a3").replacingOccurrences(of: "Gmail", with: "Drive"))
    XCTAssertEqual(ApprovalRequest.servicesSummary([gmail, gmail, calendar]), "Gmail · Calendar")
    XCTAssertEqual(ApprovalRequest.servicesSummary([gmail, calendar, drive]), "Gmail · Calendar +1")
  }

  func testSetupCode() {
    let code = SetupCode.generate()
    XCTAssertEqual(code.count, 6)
    XCTAssertTrue(code.allSatisfy(\.isASCIIDigit))
    XCTAssertEqual(SetupCode.display("482913"), "482 913")
    XCTAssertEqual(SetupCode.hello(code: "482913"), ["v": "1", "kind": "hello", "code": "482913"])
  }

  func testOnlyTheCodeOnScreenSignsTheWatchIn() {
    XCTAssertTrue(SetupCode.accepts("482913", showing: "482913"))
    // A sign-in meant for another watch, or for a code this watch replaced.
    XCTAssertFalse(SetupCode.accepts("482914", showing: "482913"))
    // Signed in, or not showing a code yet.
    XCTAssertFalse(SetupCode.accepts("482913", showing: nil))
    XCTAssertFalse(SetupCode.accepts("", showing: ""))
  }
}

final class WaitingStateTests: XCTestCase {
  private func history(approvals: String = "[]", acknowledgements: String = "[]", pending: Int = 0) throws
    -> ConversationHistory
  {
    let json = """
      {"conversation":{"id":"nyxa-1","title":"x","active_turn":{"turn_id":"t"},"pending_acknowledgements":\(pending)},
       "messages":[],"acknowledgements":\(acknowledgements),"approvals":\(approvals)}
      """
    return try WatchJSON.decoder.decode(ConversationHistory.self, from: Data(json.utf8))
  }

  func testAServiceApprovalIsDecidableOnTheWatch() throws {
    let held = try history(approvals: #"[{"id":"a1","status":"pending","summary":"POST /v1/responses"}]"#)
    XCTAssertTrue(held.waitingForApproval)
    XCTAssertFalse(held.waitingForCard)
    XCTAssertTrue(held.needsDecision)
  }

  func testAnActionCardIsDecidedOnTheIPhone() throws {
    let card = try history(acknowledgements: #"[{"id":"c1","status":"pending","summary":"Send email"}]"#, pending: 1)
    XCTAssertTrue(card.waitingForCard)
    XCTAssertFalse(card.waitingForApproval)
    let none = try history()
    XCTAssertFalse(none.needsDecision)
  }
}

final class TranscriptTests: XCTestCase {
  private func message(_ id: String, seq: Int, role: String, text: String, status: String = "completed")
    throws -> ChatMessage
  {
    let json = """
      {"id":"\(id)","seq":\(seq),"role":"\(role)","text":"\(text)","status":"\(status)",
       "created_at":"2026-11-17T10:00:00Z"}
      """
    return try WatchJSON.decoder.decode(ChatMessage.self, from: Data(json.utf8))
  }

  func testPagesMergeOldestFirstAndLaterReadsWin() throws {
    let newest = [
      try message("m3", seq: 3, role: "user", text: "And the hotel?"),
      try message("m4", seq: 4, role: "assistant", text: "", status: "streaming"),
    ]
    let earlier = [
      try message("m1", seq: 1, role: "user", text: "Tokyo trip?"),
      try message("m2", seq: 2, role: "assistant", text: "3 to 9 December."),
      try message("e1", seq: 2, role: "event", text: "Subagent settled"),
    ]
    var shown = Transcript.merge([], newest)
    XCTAssertEqual(shown.map(\.id), ["m3"], "an empty streaming reply waits")
    shown = Transcript.merge(shown, earlier)
    XCTAssertEqual(shown.map(\.id), ["m1", "m2", "m3"], "events are not shown")
    shown = Transcript.merge(shown, [try message("m4", seq: 4, role: "assistant", text: "Shibuya, 6 nights.")])
    XCTAssertEqual(shown.map(\.id), ["m1", "m2", "m3", "m4"])
    XCTAssertEqual(shown.last?.text, "Shibuya, 6 nights.")
  }

  func testAFailedReplyStillShows() throws {
    let shown = Transcript.merge([], [try message("m1", seq: 1, role: "assistant", text: "", status: "failed")])
    XCTAssertEqual(shown.count, 1)
  }
}
