import SwiftUI

/// Page 1: approvals waiting on the person (one card), then the NyxBot feed.
struct InboxPage: View {
  @Environment(WatchAppModel.self) private var model
  @State private var approvals: [ApprovalRequest] = []
  @State private var items: [FeedItem] = []
  @State private var loaded = false
  @State private var problem: String?

  var body: some View {
    List {
      PageHeader(title: "Inbox")
      if let problem {
        ProblemRow(message: problem) { Task { await load() } }
      }
      if !approvals.isEmpty {
        NavigationLink(value: WatchRoute.approvals(startAt: nil)) {
          ApprovalStackRow(approvals: approvals)
        }
        .listRowBackground(
          RoundedRectangle(cornerRadius: 20, style: .continuous)
            .fill(Color.nyxAmber.opacity(0.14))
            .overlay(
              RoundedRectangle(cornerRadius: 20, style: .continuous)
                .strokeBorder(Color.nyxAmber.opacity(0.35), lineWidth: 1)))
      }
      ForEach(items) { item in
        NavigationLink(value: WatchRoute.feedItem(item)) {
          FeedRow(item: item)
        }
      }
      if !loaded, problem == nil {
        ProgressView()
          .listRowBackground(Color.clear)
      } else if loaded, problem == nil, approvals.isEmpty, items.isEmpty {
        Text("All caught up")
          .font(.footnote)
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity)
          .listRowBackground(Color.clear)
      }
    }
    .task { await load() }
    .refreshable { await load() }
  }

  private func load() async {
    let service = model.service
    async let pending = Self.capture { try await service.pendingApprovals() }
    async let feed = Self.capture { try await service.feed(limit: 20) }
    let (approvalResult, feedResult) = await (pending, feed)
    var failure: Error?
    switch approvalResult {
    case let .success(list): approvals = list
    case let .failure(error): failure = error
    }
    switch feedResult {
    case let .success(page): items = page?.items ?? []
    case let .failure(error): failure = failure ?? error
    }
    problem = failure.map { ($0 as? WatchAPIError)?.displayMessage ?? "Couldn't load" }
    loaded = true
  }

  private static func capture<T>(_ work: () async throws -> T) async -> Result<T, Error> {
    do {
      return .success(try await work())
    } catch {
      return .failure(error)
    }
  }
}

/// "2 approvals · Gmail · Calendar".
private struct ApprovalStackRow: View {
  let approvals: [ApprovalRequest]

  var body: some View {
    HStack(spacing: 10) {
      ZStack {
        Circle().fill(Color.nyxAmber)
        Image(systemName: "checkmark.shield.fill")
          .font(.system(size: 15, weight: .semibold))
          .foregroundStyle(.black)
      }
      .frame(width: 32, height: 32)
      .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 1) {
        Text(approvals.count == 1 ? "1 approval" : "\(approvals.count) approvals")
          .font(.body.weight(.semibold))
        Text(ApprovalRequest.servicesSummary(approvals))
          .font(.footnote)
          .foregroundStyle(Color.nyxAmber)
          .lineLimit(1)
      }
      Spacer(minLength: 0)
      Image(systemName: "chevron.right")
        .font(.footnote.weight(.semibold))
        .foregroundStyle(.secondary)
    }
    .padding(.vertical, 4)
    .accessibilityElement(children: .combine)
    .accessibilityHint("Needs your decision")
  }
}

private struct FeedRow: View {
  let item: FeedItem
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    let agent = item.agent(in: model)
    ListRow(
      title: item.heading,
      subtitle: [agent?.name, RelativeTime.short(item.updatedAt)].compactMap { $0 }.joined(separator: " · "),
      unread: !item.read
    ) {
      if let agent {
        AgentBadge(id: agent.id, name: agent.name, isNyxBot: agent.isNyxBot)
      } else {
        ZStack {
          Circle().fill(Color.nyxSurface)
          Image(systemName: item.symbol)
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(item.kind == "automation_problem" ? Color.orange : Color.secondary)
        }
        .frame(width: 32, height: 32)
      }
    }
  }
}

/// One feed entry: a few lines, and a way into its chat.
struct FeedItemView: View {
  let item: FeedItem
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 8) {
        Text(item.heading)
          .font(.headline)
          .lineLimit(2)
        if let preview = item.preview, !preview.isEmpty {
          Text(SpokenText.plain(preview))
            .font(.footnote)
            .foregroundStyle(.secondary)
            .lineLimit(4)
        }
        Text(RelativeTime.short(item.updatedAt))
          .font(.caption2)
          .foregroundStyle(.secondary)
        if item.kind == "needs_decision" {
          Label("Decide on iPhone", systemImage: "iphone")
            .font(.footnote)
            .foregroundStyle(Color.nyxAmber)
        } else if let target = item.chatTarget(in: model) {
          Button("Open chat") {
            model.openChat(target)
          }
          .buttonStyle(PillButtonStyle(primary: true))
          .padding(.top, 4)
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .topBar {
      if let agent = item.agent(in: model) {
        AgentBadge(id: agent.id, name: agent.name, isNyxBot: agent.isNyxBot, size: 22)
        Text(agent.name)
          .font(.headline)
          .lineLimit(1)
      }
    }
    .task {
      guard !item.read else { return }
      try? await model.service.markRead([item.id])
    }
  }
}

/// The agent behind a feed entry, for its avatar and name.
struct FeedAgent {
  let id: String
  let name: String
  let isNyxBot: Bool
}

extension FeedItem {
  var symbol: String {
    switch kind {
    case "automation_result": return "clock.badge.checkmark"
    case "automation_problem": return "exclamationmark.triangle"
    case "needs_decision": return "hand.raised"
    case "chat_update": return "bubble.left"
    case "group_mention": return "at"
    default: return "text.bubble"
    }
  }

  @MainActor
  func agent(in model: WatchAppModel) -> FeedAgent? {
    if actor?.type == "agent", let id = actor?.id {
      return FeedAgent(id: id, name: actor?.label ?? "Agent", isNyxBot: actor?.kind == "nyxbot")
    }
    // Automations and other sources name the agent they delivered through.
    if let agent = model.agent(id: source?.agentId) {
      return FeedAgent(id: agent.id, name: agent.label, isNyxBot: agent.isNyxBot)
    }
    return nil
  }

  /// The chat this entry came from, when it has one.
  @MainActor
  func chatTarget(in model: WatchAppModel) -> ChatTarget? {
    guard let conversationId = source?.conversationId else { return nil }
    let agent = agent(in: model)
    return ChatTarget(
      agentId: agent?.id ?? "", agentName: agent?.name ?? "Chat",
      isNyxBot: agent?.isNyxBot ?? false, conversationId: conversationId)
  }
}
