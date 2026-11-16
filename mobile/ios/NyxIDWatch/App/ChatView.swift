import Observation
import SwiftUI
import UserNotifications
import WatchKit

/// Talk to one agent: send a question, follow the work as it happens, then
/// read (and, with headphones, hear) the answer.
@MainActor
@Observable
final class ChatModel {
  enum Status: Equatable {
    case loading
    case idle
    case sending
    case working
    case problem(String)

    var isBusy: Bool {
      switch self {
      case .sending, .working: return true
      default: return false
      }
    }
  }

  let target: ChatTarget
  private let service: WatchService
  private(set) var conversationId: String?
  /// The latest thing the person said, and the reply to it.
  private(set) var asked: String?
  private(set) var reply: ChatMessage?
  private(set) var status: Status = .loading
  /// "Listing events on Google Calendar…" while the agent works.
  private(set) var workingLine = "Thinking…"
  private(set) var waitingForApproval = false
  private(set) var waitingForCard = false
  private(set) var speaking = false
  private var follower: Task<Void, Never>?
  private var sentFirstMessage = false
  private var lastSeq = 0

  static let pollInterval: UInt64 = 2_000_000_000
  static let followLimit: TimeInterval = 10 * 60

  init(target: ChatTarget, service: WatchService) {
    self.target = target
    self.service = service
    self.conversationId = target.conversationId
    ReplySpeaker.shared.onChange = { [weak self] value in self?.speaking = value }
  }

  func load() async {
    do {
      if conversationId == nil, !target.agentId.isEmpty {
        conversationId = try await service.threadId(agentId: target.agentId, home: nil)
      }
      var turnActive = false
      if let id = conversationId {
        let history = try await service.history(conversationId: id, limit: 8)
        apply(history)
        turnActive = history.conversation.activeTurn != nil
      }
      if let first = target.firstMessage, !sentFirstMessage {
        sentFirstMessage = true
        status = .idle
        await send(first)
      } else if turnActive {
        status = .working
        follow(after: lastSeq, announce: false)
      } else {
        status = .idle
      }
    } catch let error as WatchAPIError {
      status = .problem(error.displayMessage)
    } catch {
      status = .problem("Couldn't open")
    }
  }

  func send(_ raw: String) async {
    let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty, !status.isBusy else { return }
    ReplySpeaker.shared.stop()
    let baseline = lastSeq
    asked = text
    reply = nil
    workingLine = "Thinking…"
    status = .sending
    let turn: TurnRequest
    if let id = conversationId {
      turn = .reply(in: id, text: text)
    } else {
      turn = .newThread(with: target.agentId, text: text)
    }
    do {
      conversationId = try await service.send(turn)
      WKInterfaceDevice.current().play(.click)
      status = .working
      follow(after: baseline, announce: true)
    } catch let error as WatchAPIError where error.errorKey == "turn_active" {
      status = .working
      follow(after: baseline, announce: false)
    } catch let error as WatchAPIError {
      status = .problem(error.displayMessage)
    } catch {
      status = .problem("Not sent")
    }
  }

  func stop() async {
    guard let id = conversationId, status.isBusy else { return }
    workingLine = "Stopping…"
    try? await service.stop(conversationId: id)
  }

  func speakReply() {
    guard let reply, !reply.failed else { return }
    ReplySpeaker.shared.speak(reply.text)
  }

  func stopSpeaking() {
    ReplySpeaker.shared.stop()
  }

  func leave() {
    follower?.cancel()
    follower = nil
    ReplySpeaker.shared.stop()
  }

  /// The web page for this chat, so the iPhone can pick it up (Handoff).
  func handoffURL(frontend: URL?) -> URL? {
    guard let frontend, let conversationId,
      var components = URLComponents(url: frontend, resolvingAgainstBaseURL: false)
    else { return nil }
    components.path += "/assistant"
    components.queryItems = [URLQueryItem(name: "c", value: conversationId)]
    return components.url
  }

  private func follow(after baseline: Int, announce: Bool) {
    follower?.cancel()
    follower = Task { [weak self] in
      let deadline = Date().addingTimeInterval(Self.followLimit)
      var unseen = 0
      while !Task.isCancelled, Date() < deadline {
        try? await Task.sleep(nanoseconds: Self.pollInterval)
        guard let self, !Task.isCancelled, let id = self.conversationId else { return }
        guard let history = try? await self.service.history(conversationId: id, limit: 8) else { continue }
        self.apply(history)
        switch TurnProgress.evaluate(history, afterSeq: baseline) {
        case .working:
          if let line = history.conversation.activeTurn?.statusLine, self.workingLine != "Stopping…" {
            self.workingLine = line
          }
          self.status = .working
        case .replied:
          self.status = .idle
          if announce { self.announceReply() }
          return
        case .failed:
          self.status = self.reply?.failed == true ? .idle : .problem("No answer")
          WKInterfaceDevice.current().play(.failure)
          return
        case .pending:
          unseen += 1
          if unseen >= 5 {
            self.status = .problem("No answer yet")
            return
          }
        }
      }
      guard let self, !Task.isCancelled else { return }
      self.status = .problem("Still working")
    }
  }

  /// "We'll tap you when it's ready": a tap, then the answer aloud when
  /// headphones are connected, or an alert if the wrist is down.
  private func announceReply() {
    WKInterfaceDevice.current().play(.notification)
    if WatchAppModel.shared.speakReplies, ReplySpeaker.headphonesConnected {
      speakReply()
    }
    guard WKApplication.shared().applicationState != .active, let reply, !reply.failed else { return }
    let content = UNMutableNotificationContent()
    content.title = target.agentName
    content.body = SpokenText.brief(reply.text).text
    content.sound = .default
    UNUserNotificationCenter.current().add(
      UNNotificationRequest(identifier: "reply-\(reply.id)", content: content, trigger: nil))
  }

  /// Keeps only the latest exchange: what was asked, and its reply.
  private func apply(_ history: ConversationHistory) {
    lastSeq = history.messages.map(\.seq).max() ?? lastSeq
    waitingForApproval = history.waitingForApproval
    waitingForCard = history.waitingForCard
    let lastPerson = history.messages.last(where: \.isFromPerson)
    if let lastPerson {
      asked = lastPerson.text
      reply = history.messages.last { $0.isReply && $0.seq > lastPerson.seq }
    } else {
      reply = history.messages.last(where: \.isReply)
    }
  }
}

struct ChatView: View {
  @Environment(WatchAppModel.self) private var app
  @State private var chat: ChatModel?
  let target: ChatTarget

  var body: some View {
    Group {
      if let chat {
        ChatContent(chat: chat)
          .userActivity(
            NSUserActivityTypeBrowsingWeb,
            isActive: chat.handoffURL(frontend: app.frontendURL) != nil
          ) { activity in
            activity.webpageURL = chat.handoffURL(frontend: app.frontendURL)
          }
      } else {
        ProgressView()
      }
    }
    .task {
      let model = chat ?? ChatModel(target: target, service: app.service)
      chat = model
      await model.load()
    }
    .onDisappear { chat?.leave() }
  }
}

private struct ChatContent: View {
  let chat: ChatModel
  @State private var showingFullReply = false
  @State private var handedOff = false

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 8) {
        content
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.bottom, 56)
    }
    .safeAreaInset(edge: .bottom, spacing: 0) {
      bottomBar
    }
    .topBar {
      AgentBadge(
        id: chat.target.agentId.isEmpty ? chat.target.agentName : chat.target.agentId,
        name: chat.target.agentName, isNyxBot: chat.target.isNyxBot, size: 22)
      Text(chat.target.agentName)
        .font(.headline)
        .lineLimit(1)
        .accessibilityAddTraits(.isHeader)
    }
    .overlay {
      if handedOff {
        Label("Continue on iPhone", systemImage: "iphone")
          .font(.footnote.weight(.semibold))
          .padding(.horizontal, 12)
          .padding(.vertical, 8)
          .background(Capsule().fill(Color.nyxSurface))
          .transition(.opacity)
      }
    }
    .sheet(isPresented: $showingFullReply) {
      ScrollView {
        Text(SpokenText.plain(chat.reply?.text ?? ""))
          .font(.footnote)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }

  @ViewBuilder
  private var content: some View {
    switch chat.status {
    case .loading:
      ProgressView()
        .frame(maxWidth: .infinity)
    case .sending, .working:
      if let asked = chat.asked {
        QuestionBubble(text: asked)
      }
      if chat.waitingForApproval {
        // The work is held by a service approval this watch can decide.
        Text("Needs your approval")
          .font(.body.weight(.semibold))
        NavigationLink(value: WatchRoute.approvals(startAt: nil)) {
          HStack(spacing: 4) {
            Text("Review")
            Image(systemName: "chevron.right")
          }
        }
        .buttonStyle(PillButtonStyle(primary: true))
      } else if chat.waitingForCard {
        Text("Needs your decision")
          .font(.body.weight(.semibold))
        Label("Decide on iPhone", systemImage: "iphone")
          .font(.footnote)
          .foregroundStyle(Color.nyxAmber)
      } else {
        TypingDots()
          .padding(.top, 2)
        Text(chat.workingLine)
          .font(.body.weight(.semibold))
          .lineLimit(2)
        Text("You can lower your wrist. We'll tap you when it's ready.")
          .font(.footnote)
          .foregroundStyle(.secondary)
      }
    case .idle:
      if let reply = chat.reply {
        ReplyBody(reply: reply) { showingFullReply = true }
      } else if let asked = chat.asked {
        QuestionBubble(text: asked)
      } else {
        Text("Tap the mic to ask")
          .font(.footnote)
          .foregroundStyle(.secondary)
      }
    case let .problem(text):
      if let reply = chat.reply {
        ReplyBody(reply: reply) { showingFullReply = true }
      }
      Label(text, systemImage: "exclamationmark.circle")
        .font(.footnote)
        .foregroundStyle(.orange)
    }
  }

  @ViewBuilder
  private var bottomBar: some View {
    HStack {
      if chat.status.isBusy {
        Spacer()
        Button {
          Task { await chat.stop() }
        } label: {
          Label("Stop", systemImage: "stop.fill")
        }
        .buttonStyle(PillButtonStyle())
        Spacer()
      } else {
        Button {
          if chat.speaking { chat.stopSpeaking() } else { chat.speakReply() }
        } label: {
          Image(systemName: chat.speaking ? "speaker.slash.fill" : "speaker.wave.2.fill")
        }
        .buttonStyle(CircleButtonStyle(size: 32))
        .disabled(chat.reply == nil)
        .accessibilityLabel(chat.speaking ? "Stop reading" : "Read aloud")
        Spacer()
        TextFieldLink(prompt: Text("Ask \(chat.target.agentName)")) {
          Image(systemName: "mic.fill")
        } onSubmit: { text in
          Task { await chat.send(text) }
        }
        .buttonStyle(CircleButtonStyle(primary: true, size: 42))
        .disabled(chat.status == .loading)
        .accessibilityLabel("Speak")
        Spacer()
        Button {
          withAnimation { handedOff = true }
          Task {
            try? await Task.sleep(nanoseconds: 1_600_000_000)
            withAnimation { handedOff = false }
          }
        } label: {
          Image(systemName: "iphone")
        }
        .buttonStyle(CircleButtonStyle(size: 32))
        .accessibilityLabel("Continue on iPhone")
      }
    }
    .padding(.horizontal, 6)
    .bottomBarBackdrop()
  }
}

/// What the person asked, right-aligned like a sent message.
private struct QuestionBubble: View {
  let text: String

  var body: some View {
    Text(text)
      .font(.footnote)
      .lineLimit(3)
      .padding(.horizontal, 10)
      .padding(.vertical, 6)
      .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color.nyxSurface))
      .frame(maxWidth: .infinity, alignment: .trailing)
  }
}

/// A headline, then the first list with times in their own column.
private struct ReplyBody: View {
  let reply: ChatMessage
  let showAll: () -> Void

  var body: some View {
    if reply.failed {
      Label("Couldn't answer", systemImage: "exclamationmark.circle")
        .font(.footnote)
        .foregroundStyle(.orange)
    } else {
      let layout = ReplyLayout.make(reply.text)
      VStack(alignment: .leading, spacing: 6) {
        if !layout.lead.isEmpty {
          Text(layout.headline)
            .font(.body.weight(.semibold))
          if let detail = layout.detail {
            Text(detail)
              .font(.footnote)
              .foregroundStyle(.secondary)
          }
        }
        ForEach(Array(layout.rows.enumerated()), id: \.offset) { _, row in
          HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let time = row.time {
              Text(time)
                .font(.footnote.monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(minWidth: 40, alignment: .leading)
            }
            Text(row.text)
              .font(.footnote)
          }
        }
        if layout.hasMore {
          Button(action: showAll) {
            Text("More ›")
              .font(.footnote.weight(.semibold))
              .foregroundStyle(Color.nyxAccent)
          }
          .buttonStyle(.plain)
          .accessibilityLabel("Show the full reply")
        }
      }
    }
  }
}
