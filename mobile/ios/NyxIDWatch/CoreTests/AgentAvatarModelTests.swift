import XCTest

@testable import NyxIDWatchCore

final class AgentAvatarModelTests: XCTestCase {
  // Expected values computed with mobile/src/components/agentAvatarModel.ts.
  func testTintMatchesThePhoneAndWeb() {
    XCTAssertEqual(AgentAvatarModel.tintIndex("99dfa222-b1a4-46bc-9687-7158c77f1f3d"), 1)
    XCTAssertEqual(AgentAvatarModel.tintIndex("ef2af606-2f43-4e9e-911e-21636811cce0"), 2)
    XCTAssertEqual(AgentAvatarModel.tintIndex("s1"), 3)
    XCTAssertEqual(AgentAvatarModel.tintIndex("agent-é"), 4)
    XCTAssertEqual(AgentAvatarModel.tint(for: "s1", isNyxBot: true), AgentAvatarModel.nyxBot)
  }

  func testInitialsMatchThePhoneAndWeb() {
    XCTAssertEqual(AgentAvatarModel.initials("Researcher"), "RE")
    XCTAssertEqual(AgentAvatarModel.initials("Travel planner"), "TP")
    XCTAssertEqual(AgentAvatarModel.initials("nyx_helper-bot"), "NH")
    XCTAssertEqual(AgentAvatarModel.initials("R2"), "R2")
    XCTAssertEqual(AgentAvatarModel.initials("—"), "—")
    XCTAssertEqual(AgentAvatarModel.initials(""), "?")
  }
}
