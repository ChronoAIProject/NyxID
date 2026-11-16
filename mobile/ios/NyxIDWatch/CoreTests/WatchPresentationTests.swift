import Foundation
import XCTest

@testable import NyxIDWatchCore

final class ReplyLayoutTests: XCTestCase {
  func testHeadlineThenTimesInTheirOwnColumn() {
    let reply = """
      3 meetings today, 2 approvals waiting.

      - 10:00 Design review · Ana
      - 13:00 Lunch · Sam
      - 16:30–17:00 Weekly sync
      """
    let layout = ReplyLayout.make(reply)
    XCTAssertEqual(layout.lead, "3 meetings today, 2 approvals waiting.")
    XCTAssertEqual(layout.rows, [
      .init(time: "10:00", text: "Design review · Ana"),
      .init(time: "13:00", text: "Lunch · Sam"),
      .init(time: "16:30–17:00", text: "Weekly sync"),
    ])
    XCTAssertFalse(layout.hasMore)
  }

  func testTheFirstSentenceIsTheHeadline() {
    let layout = ReplyLayout.make(
      "You have 3 meetings today and 2 approvals waiting. The first is the design review at 10:00.")
    XCTAssertEqual(layout.headline, "You have 3 meetings today and 2 approvals waiting.")
    XCTAssertEqual(layout.detail, "The first is the design review at 10:00.")
    XCTAssertNil(ReplyLayout.make("Done.").detail)
    XCTAssertEqual(ReplyLayout.make("Version 2.1 is out").headline, "Version 2.1 is out")
  }

  func testListsWithoutTimesAndLeftoversGoToMore() {
    let reply = "Done.\n\n1. Booked the hotel\n2) Sent the invite\n\nAnything else?"
    let layout = ReplyLayout.make(reply)
    XCTAssertEqual(layout.lead, "Done.")
    XCTAssertEqual(layout.rows.map(\.text), ["Booked the hotel", "Sent the invite"])
    XCTAssertNil(layout.rows.first?.time)
    XCTAssertTrue(layout.hasMore)
  }

  func testLongListsAndLongLeadsAreCut() {
    let items = (1...9).map { "- Item \($0)" }.joined(separator: "\n")
    let layout = ReplyLayout.make("Here you go.\n\n" + items)
    XCTAssertEqual(layout.rows.count, ReplyLayout.maxRows)
    XCTAssertTrue(layout.hasMore)

    let long = ReplyLayout.make(String(repeating: "A long sentence about the plan. ", count: 20))
    XCTAssertLessThanOrEqual(long.lead.count, SpokenText.briefLimit)
    XCTAssertTrue(long.hasMore)
  }

  func testAReplyThatIsOnlyAList() {
    let layout = ReplyLayout.make("- 9:30 AM Standup\n- 2 PM Review")
    XCTAssertEqual(layout.lead, "")
    XCTAssertEqual(layout.rows.first, .init(time: "9:30 AM", text: "Standup"))
    XCTAssertEqual(layout.rows.last, .init(time: nil, text: "2 PM Review"))
  }
}

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
