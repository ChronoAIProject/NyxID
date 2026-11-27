import Foundation

// Wire models for the handful of NyxID endpoints the watch uses. Every field
// the watch doesn't need is left out; optional fields tolerate older servers.
// The shared decoder converts snake_case keys, so properties are camelCase.

// MARK: Service approvals (`/approvals/requests`)

struct ApprovalPage: Decodable, Equatable {
  let requests: [ApprovalRequest]
  let total: Int
}

struct ApprovalRequest: Decodable, Equatable, Identifiable, Sendable {
  let id: String
  let serviceName: String
  let serviceSlug: String
  let requesterType: String
  let requesterLabel: String?
  let operationSummary: String
  let actionDescription: String?
  let httpMethod: String?
  let resource: String?
  let verb: String?
  let toolName: String?
  let isDestructive: Bool?
  let approvalMode: String
  let status: String
  let createdAt: Date
  let expiresAt: Date?
  let fromOrgPolicy: Bool?
  let orgName: String?
  /// Present only for execution-permission proposals, which need the full
  /// review page and can't be decided with a plain approve/deny.
  let executionProposal: ExecutionProposalSummary?
  let decisionDestination: String?

  var title: String {
    let name = serviceName.trimmingCharacters(in: .whitespaces)
    return name.isEmpty ? "Unknown service" : name
  }

  var summary: String { actionDescription ?? operationSummary }

  /// Who is asking: the key or app's own name when the server has one,
  /// otherwise a readable kind of caller.
  var requester: String {
    let label = requesterLabel?.trimmingCharacters(in: .whitespaces) ?? ""
    if !label.isEmpty { return label }
    switch requesterType {
    case "api_key": return "an agent key"
    case "service_account": return "a service account"
    case "access_token": return "a connected app"
    case "delegated": return "a delegated agent"
    case "user": return "you"
    default: return requesterType.replacingOccurrences(of: "_", with: " ")
    }
  }

  var risk: ApprovalRisk {
    if isDestructive == true || verb == "destructive" { return .high }
    switch verb {
    case "write": return .medium
    case "read": return .low
    default: return .unknown
    }
  }

  var requiresOwnerChatReview: Bool { decisionDestination == "originating_owner_chat" }
  var reviewCue: String {
    guard requiresOwnerChatReview else { return "Review on iPhone" }
    switch status {
    case "approved": return "Approved"
    case "rejected": return "Denied"
    case "expired": return "Expired"
    default: break
    }
    guard let expiresAt else { return "Waiting for the server" }
    return expiresAt <= Date() ? "Expired" : "Review in original NyxBot chat"
  }

  /// Absence preserves older servers; a dedicated decision destination is review-only.
  var canDecideOnWatch: Bool {
    status == "pending" && executionProposal == nil && decisionDestination == nil
  }

  var isGrant: Bool { approvalMode == "grant" }
}

struct ExecutionProposalSummary: Decodable, Equatable, Sendable {
  let state: String?
  let reviewPath: String?
}

enum ApprovalRisk: Equatable {
  case low, medium, high, unknown
}

struct ApprovalDecision: Encodable, Equatable {
  let approved: Bool
  let durationSec: Int?
}

struct ApprovalDecisionResult: Decodable, Equatable {
  let id: String
  let status: String
}

struct NotificationSettings: Decodable, Equatable {
  let grantExpiryDays: Int?
}

// MARK: NyxBot notifications feed (`/assistant/nyxagent/notifications`)

struct FeedPage: Decodable, Equatable {
  let items: [FeedItem]
  let nextCursor: String?
  let asOf: String?
}

struct FeedItem: Decodable, Equatable, Identifiable, Sendable {
  let id: String
  let kind: String
  let count: Int?
  let title: String?
  let preview: String?
  let read: Bool
  let resolved: Bool?
  let createdAt: Date?
  let updatedAt: Date
  let actor: FeedActor?
  let source: FeedSource?

  var heading: String {
    if let title, !title.isEmpty { return title }
    if let name = actor?.label { return name }
    return FeedItem.kindLabel(kind)
  }

  static func kindLabel(_ kind: String) -> String {
    switch kind {
    case "automation_result": return "Automation result"
    case "automation_problem": return "Automation problem"
    case "needs_decision": return "Needs your decision"
    case "agent_update": return "Agent update"
    case "chat_update": return "Chat update"
    case "group_mention": return "Mentioned you"
    default: return "Notification"
    }
  }
}

struct FeedActor: Decodable, Equatable, Sendable {
  let type: String?
  let id: String?
  let name: String?
  let displayName: String?
  /// `nyxbot` or `specialist` for agent actors.
  let kind: String?

  var label: String? {
    let value = (displayName?.isEmpty == false ? displayName : name)?
      .trimmingCharacters(in: .whitespaces)
    return value?.isEmpty == false ? value : nil
  }
}

struct FeedSource: Decodable, Equatable, Sendable {
  let type: String
  let id: String?
  let label: String?
  let conversationId: String?
  let agentId: String?
}

struct FeedSummary: Decodable, Equatable {
  let unreadCount: Int
  let needsDecisionCount: Int
}

struct MarkReadByIds: Encodable {
  let ids: [String]
}

/// Marks everything from one source (a thread) as seen.
struct MarkReadBySource: Encodable {
  struct Source: Encodable {
    let type: String
    let id: String
  }

  let source: Source

  static func conversation(_ id: String) -> MarkReadBySource {
    MarkReadBySource(source: Source(type: "conversation", id: id))
  }
}

struct MarkReadResult: Decodable {
  let updated: Int
}

// MARK: Account (`/users/me`)

struct UserProfile: Decodable, Equatable, Sendable {
  let email: String?
  let displayName: String?

  var name: String {
    let display = displayName?.trimmingCharacters(in: .whitespaces) ?? ""
    return display.isEmpty ? (email ?? "NyxID account") : display
  }
}

// MARK: Agents and conversations (`/assistant/nyxagent/...`)

struct AgentList: Decodable {
  let agents: [AgentSummary]
}

struct AgentSummary: Decodable, Equatable, Identifiable, Sendable {
  let id: String
  let kind: String
  let name: String
  let displayName: String?
  let status: String
  let canUse: Bool?
  let specialty: String?
  let description: String?
  let homeConversationId: String?
  let lastReply: AgentLastReply?
  let pendingAcknowledgements: Int?

  var label: String {
    let display = displayName?.trimmingCharacters(in: .whitespaces) ?? ""
    if !display.isEmpty { return display }
    return kind == "nyxbot" ? "NyxBot" : name
  }

  var isNyxBot: Bool { kind == "nyxbot" }

  /// Mirrors the iPhone app: shared agents whose access ended and destroyed
  /// agents stay listed elsewhere but can't be talked to.
  var canChat: Bool { (canUse ?? true) && status != "destroyed" }

  /// One short line under the name.
  var tagline: String {
    if isNyxBot { return "Your assistant" }
    for value in [specialty, description] {
      let text = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
      if !text.isEmpty { return text }
    }
    return "Agent"
  }
}

struct AgentLastReply: Decodable, Equatable, Sendable {
  let status: String?
  let text: String?
  let createdAt: Date?
}

struct ConversationList: Decodable {
  let conversations: [ConversationSummary]
}

struct ConversationSummary: Decodable, Equatable, Identifiable, Sendable {
  let id: String
  let title: String?
  let lastMessageAt: Date?
  let messageCount: Int?
  let agent: ConversationAgent?

  /// A thread with nothing in it yet (an agent's untouched home thread).
  var isEmpty: Bool { messageCount == 0 }

  var label: String {
    let value = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return value.isEmpty ? "New chat" : value
  }
}

/// The agent a thread belongs to (`ConversationResponse.agent`).
struct ConversationAgent: Decodable, Equatable, Sendable {
  let id: String
  let kind: String
  let name: String
  let displayName: String?
  let destroyed: Bool?

  var label: String {
    let display = displayName?.trimmingCharacters(in: .whitespaces) ?? ""
    if !display.isEmpty { return display }
    return kind == "nyxbot" ? "NyxBot" : name
  }
}

struct ConversationHistory: Decodable, Equatable, Sendable {
  let conversation: ConversationState
  let messages: [ChatMessage]
  /// Set when older messages exist: pass it back to load them.
  let beforeSeq: Int?
  let acknowledgements: [PendingCard]?
  let approvals: [PendingCard]?

  /// The person has to decide something before this thread can continue.
  var needsDecision: Bool { waitingForApproval || waitingForCard }

  /// A service approval holds the thread; the watch can decide it.
  var waitingForApproval: Bool {
    approvals?.contains(where: { $0.status == nil || $0.status == "pending" }) == true
  }

  /// An agent's action card holds the thread; it is decided on the iPhone.
  var waitingForCard: Bool {
    if (conversation.pendingAcknowledgements ?? 0) > 0 { return true }
    return acknowledgements?.contains(where: { $0.status == "pending" }) == true
  }
}

struct ConversationState: Decodable, Equatable, Sendable {
  let id: String
  let title: String?
  let activeTurn: ActiveTurn?
  let pendingAcknowledgements: Int?
}

struct ActiveTurn: Decodable, Equatable, Sendable {
  let turnId: String
  let reply: StreamingReply?
  let queued: Bool?
  let waitingForCapacity: Bool?
  let stopRequested: Bool?
  /// Tool calls so far (identifiers only, never arguments).
  let activities: [TurnActivityInfo]?

  /// What the agent is doing right now, in a few words.
  var statusLine: String {
    if queued == true || waitingForCapacity == true { return "Waiting for a free slot…" }
    if let running = activities?.last(where: { $0.status == "running" }) {
      return ToolActivityText.running(running.label) + "…"
    }
    return "Thinking…"
  }
}

struct TurnActivityInfo: Decodable, Equatable, Sendable {
  let label: String
  let status: String
}

struct StreamingReply: Decodable, Equatable, Sendable {
  let text: String?
}

struct ChatMessage: Decodable, Equatable, Identifiable, Sendable {
  let id: String
  let seq: Int
  let turnId: String?
  let role: String
  let text: String
  let status: String
  let errorCode: String?
  let errorDetail: ChatErrorDetail?
  let createdAt: Date

  var isFromPerson: Bool { role == "user" }
  var isReply: Bool { role == "assistant" }
  var failed: Bool { status == "failed" }
}

struct ChatErrorDetail: Decodable, Equatable, Sendable {
  let message: String?
}

struct PendingCard: Decodable, Equatable, Sendable {
  let id: String
  let status: String?
  let summary: String?
}

/// `POST /assistant/nyxagent/turns`: exactly one of the two targets.
struct TurnRequest: Encodable, Equatable {
  let conversationId: String?
  let agentId: String?
  let text: String

  static func reply(in conversationId: String, text: String) -> TurnRequest {
    TurnRequest(conversationId: conversationId, agentId: nil, text: text)
  }

  static func newThread(with agentId: String, text: String) -> TurnRequest {
    TurnRequest(conversationId: nil, agentId: agentId, text: text)
  }

  private enum CodingKeys: String, CodingKey {
    case conversationId, agentId, text
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    // The server rejects unknown keys and both targets at once, so omit nils.
    try container.encodeIfPresent(conversationId, forKey: .conversationId)
    try container.encodeIfPresent(agentId, forKey: .agentId)
    try container.encode(text, forKey: .text)
  }
}
