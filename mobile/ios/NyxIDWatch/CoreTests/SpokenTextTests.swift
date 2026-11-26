import XCTest

@testable import NyxIDWatchCore

final class SpokenTextTests: XCTestCase {
  func testPlainTextDropsMarkdownSyntax() {
    let markdown = """
      ## Summary
      You have **3** new emails and a [calendar invite](https://cal.example.com/x).

      - Reply to *Ana*
      - [x] Pay `invoice-42`
      > Quoted line

      ```swift
      print("hidden")
      ```
      ---
      | Name | Count |
      | --- | ---: |
      | Mail | 3 |
      """
    XCTAssertEqual(
      SpokenText.plain(markdown),
      """
      Summary
      You have 3 new emails and a calendar invite.

      • Reply to Ana
      • Pay invoice-42
      Quoted line

      [code]

      Name, Count
      Mail, 3
      """
    )
  }

  func testSpeechNamesLinksAndCodeInsteadOfReadingThem() {
    let spoken = SpokenText.prepare("See https://example.com/a?b=c for details.\n```\nx\n```")
    XCTAssertEqual(spoken, "See a link for details. There's code on screen.")
  }

  func testShortRepliesAreShownWhole() {
    XCTAssertEqual(
      SpokenText.brief("**Done.** Sent the email."),
      SpokenText.Brief(text: "Done. Sent the email.", truncated: false))
  }

  func testLongRepliesStopAtAParagraphOrSentence() {
    let paragraph = String(repeating: "This sentence is about forty characters. ", count: 4)
    let reply = paragraph + "\n\n" + String(repeating: "More detail follows here. ", count: 20)
    let brief = SpokenText.brief(reply)
    XCTAssertTrue(brief.truncated)
    XCTAssertEqual(brief.text, paragraph.trimmingCharacters(in: .whitespaces))
    XCTAssertLessThanOrEqual(brief.text.count, SpokenText.briefLimit)

    let run = SpokenText.brief(String(repeating: "Forty characters of one long sentence. ", count: 12))
    XCTAssertTrue(run.text.hasSuffix("sentence."))
    XCTAssertTrue(SpokenText.prepare(reply).hasSuffix("More on screen."))
  }

  func testOnlyTheFirstParagraphIsShownAndAHeadingBringsTheNextOne() {
    let reply = "## Today\n\nYou have three meetings and two approvals waiting.\n\n- 10:00 Design review\n- 13:00 Lunch"
    XCTAssertEqual(
      SpokenText.brief(reply),
      SpokenText.Brief(text: "Today\n\nYou have three meetings and two approvals waiting.", truncated: true))
    XCTAssertEqual(
      SpokenText.prepare(reply),
      "Today You have three meetings and two approvals waiting. More on screen.")
  }

  func testUnbrokenTextIsCutAtAWord() {
    let brief = SpokenText.brief(String(repeating: "word ", count: 80))
    XCTAssertTrue(brief.text.hasSuffix("word…"))
    XCTAssertFalse(SpokenText.prepare(String(repeating: "word ", count: 80)).contains("…"))
  }

  func testSnakeCaseIdentifiersSurvive() {
    XCTAssertEqual(SpokenText.plain("Use nyx__call_tool now"), "Use nyx__call_tool now")
  }
}
