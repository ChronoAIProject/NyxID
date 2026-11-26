import Foundation
import XCTest

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

@testable import NyxIDWatchCore

final class WatchAPITests: XCTestCase {
  func testRedeemStoresTheWatchsOwnSessionAndSendsAttribution() async throws {
    let transport = FakeTransport()
    transport.on(
      "POST", "/api/v1/auth/login-code/redeem",
      json: #"{"auth_kind":"account_session","access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":900}"#)
    let storage = MemorySessionStorage()
    let now = Date(timeIntervalSince1970: 1_000)
    let api = WatchAPI(storage: storage, transport: transport, now: { now })

    try await api.redeem(code: "ABCD-EFGH", apiBaseURL: Fixtures.base, context: WatchClientInfo.loginContext())

    let stored = try XCTUnwrap(storage.load())
    XCTAssertEqual(stored.accessToken, "a")
    XCTAssertEqual(stored.refreshToken, "r")
    XCTAssertEqual(stored.accessExpiresAt, now.addingTimeInterval(900))
    let request = try XCTUnwrap(transport.calls("POST", "/api/v1/auth/login-code/redeem").first)
    XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
    XCTAssertEqual(request.jsonBody["code"] as? String, "ABCD-EFGH")
    XCTAssertEqual(request.jsonBody["client_label"] as? String, "Apple Watch")
    XCTAssertEqual(request.jsonBody["client_form_factor"] as? String, "mobile")
    XCTAssertEqual(request.value(forHTTPHeaderField: "X-NyxID-Client"), "mobile")
    let isSignedIn = await api.isSignedIn
    XCTAssertTrue(isSignedIn)
  }

  func testRedeemRefusesAnAgentKeyDelivery() async {
    let transport = FakeTransport()
    transport.on(
      "POST", "/api/v1/auth/login-code/redeem",
      json: #"{"auth_kind":"agent_key","credential":"nyxid_ag_x","credential_id":"c"}"#)
    let storage = MemorySessionStorage()
    let api = WatchAPI(storage: storage, transport: transport)
    do {
      try await api.redeem(code: "ABCD-EFGH", apiBaseURL: Fixtures.base, context: LoginClientContext(fields: [:]))
      XCTFail("expected a refusal")
    } catch {
      XCTAssertNil(storage.load())
    }
  }

  func testRedeemSurfacesExpiredCodes() async {
    let transport = FakeTransport()
    transport.on(
      "POST", "/api/v1/auth/login-code/redeem", status: 410,
      json: #"{"error":"login_code_expired","error_code":12001,"message":"This code has expired"}"#)
    let api = WatchAPI(storage: MemorySessionStorage(), transport: transport)
    do {
      try await api.redeem(code: "ABCD-EFGH", apiBaseURL: Fixtures.base, context: LoginClientContext(fields: [:]))
      XCTFail("expected an error")
    } catch let error as WatchAPIError {
      XCTAssertEqual(error.errorKey, "login_code_expired")
      XCTAssertEqual(error.errorCode, 12001)
    } catch {
      XCTFail("unexpected \(error)")
    }
  }

  func testRefreshesBeforeExpiryAndKeepsTheRotatedToken() async throws {
    let now = Date(timeIntervalSince1970: 5_000)
    let storage = MemorySessionStorage(Fixtures.session(expiresIn: 30, now: now))
    let transport = FakeTransport()
    transport.on("POST", "/api/v1/auth/refresh") { request in
      XCTAssertEqual(request.jsonBody["refresh_token"] as? String, "refresh-1")
      XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
      return .init(status: 200, body: #"{"access_token":"access-2","expires_in":900,"refresh_token":"refresh-2"}"#)
    }
    transport.on("GET", "/api/v1/assistant/nyxagent/agents") { request in
      XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer access-2")
      return .init(status: 200, body: #"{"agents":[]}"#)
    }
    let api = WatchAPI(storage: storage, transport: transport, now: { now })

    let list: AgentList = try await api.get("/assistant/nyxagent/agents")

    XCTAssertTrue(list.agents.isEmpty)
    XCTAssertEqual(storage.load()?.refreshToken, "refresh-2")
    XCTAssertEqual(storage.load()?.accessToken, "access-2")
  }

  func testConcurrentRequestsShareOneRefresh() async throws {
    let now = Date(timeIntervalSince1970: 5_000)
    let storage = MemorySessionStorage(Fixtures.session(expiresIn: 10, now: now))
    let transport = FakeTransport()
    transport.delay = 5_000_000
    transport.on(
      "POST", "/api/v1/auth/refresh",
      json: #"{"access_token":"access-2","expires_in":900,"refresh_token":"refresh-2"}"#)
    transport.on("GET", "/api/v1/assistant/nyxagent/agents", json: #"{"agents":[]}"#)
    let api = WatchAPI(storage: storage, transport: transport, now: { now })

    async let first: AgentList = api.get("/assistant/nyxagent/agents")
    async let second: AgentList = api.get("/assistant/nyxagent/agents")
    async let third: AgentList = api.get("/assistant/nyxagent/agents")
    _ = try await (first, second, third)

    // Replaying a rotated refresh token would revoke the whole session.
    XCTAssertEqual(transport.calls("POST", "/api/v1/auth/refresh").count, 1)
  }

  func testA401RefreshesAndRetriesOnce() async throws {
    let storage = MemorySessionStorage(Fixtures.session())
    let transport = FakeTransport()
    transport.on("GET", "/api/v1/approvals/requests") { request in
      if request.value(forHTTPHeaderField: "Authorization") == "Bearer access-1" {
        return .init(status: 401, body: #"{"error":"unauthorized","error_code":1001,"message":"expired"}"#)
      }
      return .init(status: 200, body: #"{"requests":[],"total":0,"page":1,"per_page":50}"#)
    }
    transport.on(
      "POST", "/api/v1/auth/refresh",
      json: #"{"access_token":"access-2","expires_in":900,"refresh_token":"refresh-2"}"#)
    let api = WatchAPI(storage: storage, transport: transport)

    let approvals = try await WatchService(api: api).pendingApprovals()

    XCTAssertTrue(approvals.isEmpty)
    XCTAssertEqual(transport.calls("GET", "/api/v1/approvals/requests").count, 2)
  }

  func testRevokedRefreshSignsOutAndNotifies() async throws {
    let now = Date(timeIntervalSince1970: 5_000)
    let storage = MemorySessionStorage(Fixtures.session(expiresIn: 0, now: now))
    let transport = FakeTransport()
    transport.on(
      "POST", "/api/v1/auth/refresh", status: 401,
      json: #"{"error":"unauthorized","error_code":1001,"message":"Session revoked","details":{"reason":"session_revoked"}}"#)
    let api = WatchAPI(storage: storage, transport: transport, now: { now })
    let notified = expectation(description: "signed out")
    await api.onSignedOut { notified.fulfill() }

    do {
      let _: AgentList = try await api.get("/assistant/nyxagent/agents")
      XCTFail("expected sign-out")
    } catch let error as WatchAPIError {
      XCTAssertEqual(error, .signedOut)
    }
    await fulfillment(of: [notified], timeout: 1)
    XCTAssertNil(storage.load())
    XCTAssertTrue(transport.calls("GET", "/api/v1/assistant/nyxagent/agents").isEmpty)
  }

  func testConcurrentRotationConflictIsRetriedOnce() async throws {
    let now = Date(timeIntervalSince1970: 5_000)
    let storage = MemorySessionStorage(Fixtures.session(expiresIn: 0, now: now))
    let transport = FakeTransport()
    var attempts = 0
    transport.on("POST", "/api/v1/auth/refresh") { _ in
      attempts += 1
      if attempts == 1 {
        return .init(status: 409, body: #"{"error":"conflict","error_code":1004,"message":"Refresh token was concurrently rotated, please retry"}"#)
      }
      return .init(status: 200, body: #"{"access_token":"access-2","expires_in":900,"refresh_token":"refresh-2"}"#)
    }
    transport.on("GET", "/api/v1/assistant/nyxagent/agents", json: #"{"agents":[]}"#)
    let api = WatchAPI(storage: storage, transport: transport, now: { now })

    let _: AgentList = try await api.get("/assistant/nyxagent/agents")

    XCTAssertEqual(attempts, 2)
    XCTAssertEqual(storage.load()?.refreshToken, "refresh-2")
  }

  func testSignOutEndsOnlyThisSessionOnTheServer() async throws {
    let storage = MemorySessionStorage(Fixtures.session())
    let transport = FakeTransport()
    transport.on("POST", "/api/v1/auth/logout", json: #"{"message":"Logged out successfully"}"#)
    let api = WatchAPI(storage: storage, transport: transport)

    await api.signOut()

    XCTAssertNil(storage.load())
    let logout = try XCTUnwrap(transport.calls("POST", "/api/v1/auth/logout").first)
    XCTAssertEqual(logout.value(forHTTPHeaderField: "Authorization"), "Bearer access-1")
    XCTAssertNil(logout.httpBody)
  }

  func testDatesWithMicrosecondsAndOffsetsParse() {
    XCTAssertEqual(
      WatchJSON.parseDate("2026-11-16T10:00:00.123456+00:00")?.timeIntervalSince1970 ?? 0,
      WatchJSON.parseDate("2026-11-16T10:00:00.123Z")?.timeIntervalSince1970 ?? 1,
      accuracy: 0.001)
    XCTAssertNotNil(WatchJSON.parseDate("2026-11-16T10:00:00Z"))
    XCTAssertNotNil(WatchJSON.parseDate("2026-11-16T10:00:00.123456789Z"))
    XCTAssertNil(WatchJSON.parseDate("yesterday"))
  }
}
