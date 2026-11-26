import SwiftUI

/// Page 2: the person's agents, with a dot where something is unread. Tap
/// one to start a new chat or carry on in one of its threads.
struct AgentsPage: View {
  @Environment(WatchAppModel.self) private var model
  @State private var problem: String?
  @State private var loaded = false

  var body: some View {
    List {
      PageHeader(title: "Agents")
      if let problem {
        ProblemRow(message: problem) { Task { await load() } }
      }
      ForEach(model.agents) { agent in
        NavigationLink(value: WatchRoute.agent(agent.id)) {
          ListRow(
            title: agent.label, subtitle: agent.tagline,
            unread: model.unreadAgentIds.contains(agent.id)
          ) {
            AgentBadge(id: agent.id, name: agent.label, isNyxBot: agent.isNyxBot)
          }
        }
      }
      if !loaded, model.agents.isEmpty, problem == nil {
        ProgressView()
          .listRowBackground(Color.clear)
      }
    }
    .task { await load() }
    // Coming back from a chat: dots may have cleared.
    .onAppear { if loaded { Task { await model.refreshUnread() } } }
    .refreshable { await load() }
  }

  private func load() async {
    await model.refresh()
    loaded = true
    problem = model.agents.isEmpty ? "Couldn't load" : nil
  }
}

/// One agent: New chat first, then its threads, newest first.
struct AgentView: View {
  let agentId: String
  @Environment(WatchAppModel.self) private var model
  @State private var threads: [ConversationSummary]?
  @State private var problem: String?

  private var agent: AgentSummary? { model.agent(id: agentId) }

  var body: some View {
    List {
      Button {
        model.ask(agentId: agentId, fresh: true)
      } label: {
        HStack(spacing: 10) {
          ZStack {
            Circle().fill(Color.white)
            Image(systemName: "plus")
              .font(.system(size: 15, weight: .bold))
              .foregroundStyle(.black)
          }
          .frame(width: 32, height: 32)
          .accessibilityHidden(true)
          Text("New chat")
            .font(.body.weight(.semibold))
          Spacer(minLength: 0)
        }
        .padding(.vertical, 4)
      }
      if let problem {
        ProblemRow(message: problem) { Task { await load() } }
      }
      if let threads {
        if threads.isEmpty {
          Text("No chats yet")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
        }
        ForEach(threads) { thread in
          NavigationLink(value: WatchRoute.chat(target(for: thread))) {
            ThreadRow(
              title: thread.label,
              age: thread.lastMessageAt.map { RelativeTime.short($0) },
              unread: model.unreadThreadIds.contains(thread.id))
          }
        }
      } else if problem == nil {
        ProgressView()
          .listRowBackground(Color.clear)
      }
    }
    .topBar {
      if let agent {
        Text(agent.label)
          .font(.headline)
      }
    }
    .task { await load() }
    // Coming back from a chat: a new thread, a newer reply, a cleared dot.
    .onAppear { if threads != nil { Task { await load() } } }
    .refreshable { await load() }
  }

  private func target(for thread: ConversationSummary) -> ChatTarget {
    ChatTarget(
      agentId: agentId, agentName: agent?.label ?? thread.agent?.label ?? "Chat",
      isNyxBot: agent?.isNyxBot ?? false, conversationId: thread.id)
  }

  private func load() async {
    async let unread: Bool = model.refreshUnread()
    do {
      threads = try await model.service.threads(agentId: agentId)
      problem = nil
    } catch let failure as WatchAPIError {
      problem = failure.displayMessage
    } catch {
      problem = "Couldn't load"
    }
    _ = await unread
  }
}

/// A thread: its title (two lines), how long ago, and an unread dot.
private struct ThreadRow: View {
  let title: String
  let age: String?
  let unread: Bool

  var body: some View {
    HStack(alignment: .top, spacing: 8) {
      VStack(alignment: .leading, spacing: 2) {
        Text(title)
          .font(.body.weight(.semibold))
          .lineLimit(2)
        if let age {
          Text(age)
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
      }
      Spacer(minLength: 0)
      if unread {
        Circle()
          .fill(Color.nyxAccent)
          .frame(width: 7, height: 7)
          .padding(.top, 6)
          .accessibilityLabel("Unread")
      }
    }
    .padding(.vertical, 4)
    .accessibilityElement(children: .combine)
  }
}
