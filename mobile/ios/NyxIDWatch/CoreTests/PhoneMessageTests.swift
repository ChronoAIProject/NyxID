import XCTest

@testable import NyxIDWatchCore

final class PhoneMessageTests: XCTestCase {
  func testPairMessageNormalisesTheAPIBaseURL() {
    let message = PhoneMessage([
      "v": "1",
      "kind": "pair",
      "code": "ABCD-EFGH",
      "api_base_url": "https://auth.example.com/api/v1/",
      "frontend_url": "https://app.example.com/",
      "match": "482913",
      "expires_at": "2026-11-16T12:05:00Z",
    ])
    guard case let .pair(code, url, match, frontend, expiresAt)? = message else {
      return XCTFail("expected a pair message")
    }
    XCTAssertEqual(code, "ABCD-EFGH")
    XCTAssertEqual(url.absoluteString, "https://auth.example.com/api/v1")
    XCTAssertEqual(match, "482913")
    XCTAssertEqual(frontend?.absoluteString, "https://app.example.com")
    XCTAssertEqual(expiresAt, WatchJSON.parseDate("2026-11-16T12:05:00Z"))
  }

  func testPairingNeedsTheWatchsSetupCode() {
    let base: [String: Any] = ["v": "1", "kind": "pair", "code": "ABCD-EFGH", "api_base_url": "https://a.example/api/v1"]
    XCTAssertNil(PhoneMessage(base))
    XCTAssertNil(PhoneMessage(base.merging(["match": "12345"]) { $1 }))
    XCTAssertNil(PhoneMessage(base.merging(["match": "12a456"]) { $1 }))
    XCTAssertNotNil(PhoneMessage(base.merging(["match": "123456"]) { $1 }))
    // An untrusted web address is dropped, not fatal.
    guard case let .pair(_, _, _, frontend, _)? = PhoneMessage(
      base.merging(["match": "123456", "frontend_url": "http://evil.example"]) { $1 })
    else { return XCTFail("expected a pair message") }
    XCTAssertNil(frontend)
  }

  func testSignOutMessage() {
    XCTAssertEqual(PhoneMessage(["v": "1", "kind": "sign_out"]), .signOut)
  }

  func testRejectsUnknownVersionsAndKinds() {
    XCTAssertNil(PhoneMessage(["v": "2", "kind": "sign_out"]))
    XCTAssertNil(PhoneMessage(["kind": "sign_out"]))
    XCTAssertNil(PhoneMessage(["v": "1", "kind": "share_tokens", "token": "x"]))
  }

  func testRejectsCleartextRemoteServersAndOddURLs() {
    func pair(_ url: String, code: String = "ABCD-EFGH") -> PhoneMessage? {
      PhoneMessage(["v": "1", "kind": "pair", "code": code, "api_base_url": url, "match": "000111"])
    }
    XCTAssertNil(pair("http://auth.example.com/api/v1"))
    XCTAssertNil(pair("https://user:pass@auth.example.com/api/v1"))
    XCTAssertNil(pair("https://auth.example.com/api/v1?x=1"))
    XCTAssertNil(pair("ftp://auth.example.com"))
    XCTAssertNil(pair("not a url"))
    XCTAssertNotNil(pair("http://localhost:3001/api/v1"))
    XCTAssertNotNil(pair("http://127.0.0.1:3001/api/v1"))
  }

  func testRejectsImplausibleCodes() {
    func pair(code: String) -> PhoneMessage? {
      PhoneMessage(["v": "1", "kind": "pair", "code": code, "api_base_url": "https://a.example/api/v1", "match": "000111"])
    }
    XCTAssertNil(pair(code: ""))
    XCTAssertNil(pair(code: "abc"))
    XCTAssertNil(pair(code: "ABCD EFGH"))
    XCTAssertNil(pair(code: String(repeating: "A", count: 65)))
    XCTAssertNotNil(pair(code: "2-ABCD-EFGH"))
  }
}
