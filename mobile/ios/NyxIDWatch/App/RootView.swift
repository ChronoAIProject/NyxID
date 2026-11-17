import SwiftUI

struct RootView: View {
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    switch model.phase {
    case .loading, .signingIn:
      ProgressView()
    case let .signedOut(reason):
      SetUpView(code: model.setupCode, reason: reason)
    case .signedIn:
      MainView()
    }
  }
}

/// Shown until the iPhone app hands the watch a sign-in. The iPhone shows
/// the same code, and only this watch, showing it, can be signed in.
struct SetUpView: View {
  let code: String?
  let reason: String?

  var body: some View {
    ScrollView {
      VStack(spacing: 6) {
        Image("NyxMark")
          .resizable()
          .scaledToFit()
          .frame(width: 34, height: 34)
          .accessibilityHidden(true)
        Text("Connect to iPhone")
          .font(.headline)
        Text("Open NyxID on your iPhone, then go to Settings › Apple Watch.")
          .font(.footnote)
          .foregroundStyle(.secondary)
          .multilineTextAlignment(.center)
        if let reason {
          Text(reason)
            .font(.footnote)
            .foregroundStyle(.orange)
        }
        if let code {
          HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text("CODE")
              .font(.caption2.weight(.semibold))
              .foregroundStyle(.secondary)
            Text(SetupCode.display(code))
              .font(.system(.title3, design: .rounded).weight(.semibold).monospacedDigit())
              .kerning(2)
          }
          .padding(.top, 6)
          .accessibilityElement(children: .ignore)
          .accessibilityLabel("Code \(code.map(String.init).joined(separator: " "))")
        }
      }
      .frame(maxWidth: .infinity)
    }
  }
}

/// Unread and Agents as vertical pages (Digital Crown or swipe).
struct MainView: View {
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    @Bindable var model = model
    NavigationStack(path: $model.path) {
      TabView(selection: $model.page) {
        UnreadPage()
          .tag(WatchPage.unread)
        AgentsPage()
          .tag(WatchPage.agents)
      }
      .tabViewStyle(.verticalPage)
      .navigationDestination(for: WatchRoute.self) { route in
        switch route {
        case let .approvals(startAt): ApprovalQueueView(startAt: startAt)
        case let .feedItem(item): FeedItemView(item: item)
        case let .agent(id): AgentView(agentId: id)
        case let .ask(agentId, fresh): AskView(agentId: agentId, fresh: fresh)
        case let .chat(target): ChatView(target: target)
        }
      }
    }
    .sheet(isPresented: $model.showingSettings) {
      SettingsView()
    }
  }
}

struct SettingsView: View {
  @Environment(WatchAppModel.self) private var model
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    @Bindable var model = model
    NavigationStack {
      List {
        Toggle(isOn: $model.speakReplies) {
          VStack(alignment: .leading, spacing: 1) {
            Text("Read replies aloud")
              .font(.body.weight(.semibold))
            Text("Only with headphones")
              .font(.footnote)
              .foregroundStyle(.secondary)
          }
        }
        .tint(.nyxAccent)
        VStack(alignment: .leading, spacing: 1) {
          Text(model.profile?.name ?? "NyxID account")
            .font(.body.weight(.semibold))
            .lineLimit(1)
          Text("Signed in on this watch")
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
        Button(role: .destructive) {
          dismiss()
          Task { await model.signOut() }
        } label: {
          Label("Sign out", systemImage: "rectangle.portrait.and.arrow.right")
            .foregroundStyle(.red)
        }
      }
      .navigationTitle("Settings")
    }
  }
}
