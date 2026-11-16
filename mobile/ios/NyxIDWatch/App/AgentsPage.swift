import SwiftUI

/// Page 2: the person's agents. Tap one to ask it something.
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
        Button {
          model.ask(agentId: agent.id)
        } label: {
          ListRow(title: agent.label, subtitle: agent.tagline) {
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
    .refreshable { await load() }
  }

  private func load() async {
    await model.refresh()
    loaded = true
    problem = model.agents.isEmpty ? "Couldn't load" : nil
  }
}

/// Page 3: recent chats across agents. Tap one to continue it.
struct ChatsPage: View {
  @Environment(WatchAppModel.self) private var model
  @State private var chats: [ConversationSummary]?
  @State private var problem: String?

  var body: some View {
    List {
      PageHeader(title: "Chats")
      if let problem {
        ProblemRow(message: problem) { Task { await load() } }
      }
      if let chats {
        if chats.isEmpty {
          Text("No chats yet")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
        }
        ForEach(chats) { chat in
          let agent = chat.agent
          NavigationLink(
            value: WatchRoute.chat(
              ChatTarget(
                agentId: agent?.id ?? "", agentName: agent?.label ?? "Chat",
                isNyxBot: agent?.kind == "nyxbot", conversationId: chat.id))
          ) {
            ListRow(title: chat.label, subtitle: chat.lastMessageAt.map { RelativeTime.short($0) }) {
              AgentBadge(
                id: agent?.id ?? chat.id, name: agent?.label ?? "Chat",
                isNyxBot: agent?.kind == "nyxbot")
            }
          }
        }
      } else if problem == nil {
        ProgressView()
          .listRowBackground(Color.clear)
      }
    }
    .task { await load() }
    .refreshable { await load() }
  }

  private func load() async {
    do {
      chats = try await model.service.recentChats()
      problem = nil
    } catch let failure as WatchAPIError {
      problem = failure.displayMessage
    } catch {
      problem = "Couldn't load"
    }
  }
}
