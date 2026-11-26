import SwiftUI

/// "Tap to speak": ask an agent something. The question goes to the agent's
/// own thread, or to a new one (`fresh`), and the chat opens to follow it.
struct AskView: View {
  let agentId: String?
  var fresh = false

  @Environment(WatchAppModel.self) private var model
  @State private var selectedId: String?
  @State private var choosingAgent = false

  private var agent: AgentSummary? {
    model.agent(id: selectedId ?? agentId) ?? model.nyxBot
  }

  var body: some View {
    VStack(spacing: 6) {
      Spacer(minLength: 0)
      TextFieldLink(prompt: Text("Ask \(agent?.label ?? "")")) {
        ZStack {
          Circle()
            .strokeBorder(Color.white.opacity(0.18), lineWidth: 6)
            .frame(width: 86, height: 86)
          Circle()
            .fill(Color.white)
            .frame(width: 70, height: 70)
          Image(systemName: "mic.fill")
            .font(.system(size: 28, weight: .semibold))
            .foregroundStyle(.black)
        }
      } onSubmit: { text in
        send(text)
      }
      .buttonStyle(.plain)
      .accessibilityLabel("Speak")
      Text("Tap to speak")
        .font(.footnote)
        .foregroundStyle(.secondary)
      Spacer(minLength: 0)
      HStack(spacing: 6) {
        Button(suggestion) {
          send(suggestion)
        }
        .buttonStyle(PillButtonStyle())
        TextFieldLink(prompt: Text("Ask \(agent?.label ?? "")")) {
          Image(systemName: "keyboard")
        } onSubmit: { text in
          send(text)
        }
        .buttonStyle(CircleButtonStyle(size: 32))
        .accessibilityLabel("Type instead")
      }
    }
    .topBar {
      Button {
        choosingAgent = true
      } label: {
        HStack(spacing: 4) {
          Text(agent?.label ?? "Agent")
            .font(.footnote.weight(.semibold))
            .lineLimit(1)
            .minimumScaleFactor(0.75)
          Image(systemName: "chevron.down")
            .font(.caption2.weight(.bold))
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(Capsule().fill(Color.nyxSurface))
      }
      .buttonStyle(.plain)
      .accessibilityLabel("Talking to \(agent?.label ?? "an agent"). Change")
    }
    .sheet(isPresented: $choosingAgent) {
      AgentPicker(selectedId: agent?.id) { picked in
        selectedId = picked
        choosingAgent = false
      }
    }
  }

  /// One ready-made question.
  private var suggestion: String {
    agent?.isNyxBot == false ? "What's new?" : "Plan my day"
  }

  private func send(_ raw: String) {
    let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty, let agent else { return }
    model.openChat(model.chatTarget(for: agent, firstMessage: text, fresh: fresh))
  }
}

private struct AgentPicker: View {
  let selectedId: String?
  let pick: (String) -> Void
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    List(model.agents) { agent in
      Button {
        pick(agent.id)
      } label: {
        ListRow(title: agent.label, subtitle: agent.id == selectedId ? "Talking now" : agent.tagline) {
          AgentBadge(id: agent.id, name: agent.label, isNyxBot: agent.isNyxBot)
        }
      }
    }
  }
}
