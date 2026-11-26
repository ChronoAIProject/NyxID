import Observation
import SwiftUI
import UserNotifications
import WatchKit

/// One thread with an agent: the whole conversation, newest at the bottom,
/// older messages on request. Send a message, follow the work as it
/// happens, then read (and, with headphones, hear) the answer.
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
  /// Questions and replies, oldest first.
  private(set) var messages: [ChatMessage] = []
  /// Cursor for the page before the oldest loaded message.
  private(set) var earlierCursor: Int?
  private(set) var loadingEarlier = false
  /// Just sent, not yet in the server's history.
  private(set) var pendingQuestion: String?
  private(set) var status: Status = .loading
  /// "Listing events on Google Calendar…" while the agent works.
  private(set) var workingLine = "Thinking…"
  private(set) var waitingForApproval = false
  private(set) var waitingForCard = false
  private(set) var speaking = false
  private var follower: Task<Void, Never>?
  private var sentFirstMessage = false
  private var lastSeq = 0

  static let pageSize = 30
  static let pollInterval: UInt64 = 2_000_000_000
  static let followLimit: TimeInterval = 10 * 60

  init(target: ChatTarget, service: WatchService) {
    self.target = target
    self.service = service
    self.conversationId = target.conversationId
    ReplySpeaker.shared.onChange = { [weak self] value in self?.speaking = value }
  }

  var latestReply: ChatMessage? { messages.last(where: \.isReply) }
  var isEmpty: Bool { messages.isEmpty && pendingQuestion == nil }

  func load() async {
    do {
      if conversationId == nil, !target.fresh, !target.agentId.isEmpty {
        conversationId = try await service.threadId(agentId: target.agentId, home: nil)
      }
      var turnActive = false
      if let id = conversationId {
        let history = try await service.history(conversationId: id, limit: Self.pageSize)
        earlierCursor = history.beforeSeq
        apply(history)
        turnActive = history.conversation.activeTurn != nil
        try? await service.markThreadRead(id)
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

  func loadEarlier() async {
    guard let id = conversationId, let cursor = earlierCursor, !loadingEarlier else { return }
    loadingEarlier = true
    defer { loadingEarlier = false }
    guard let page = try? await service.history(conversationId: id, limit: Self.pageSize, beforeSeq: cursor)
    else { return }
    earlierCursor = page.beforeSeq
    messages = Transcript.merge(messages, page.messages)
  }

  func send(_ raw: String) async {
    let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty, !status.isBusy else { return }
    ReplySpeaker.shared.stop()
    let baseline = lastSeq
    pendingQuestion = text
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
      pendingQuestion = nil
      status = .working
      follow(after: baseline, announce: false)
    } catch let error as WatchAPIError {
      pendingQuestion = nil
      status = .problem(error.displayMessage)
    } catch {
      pendingQuestion = nil
      status = .problem("Not sent")
    }
  }

  func stop() async {
    guard let id = conversationId, status.isBusy else { return }
    workingLine = "Stopping…"
    try? await service.stop(conversationId: id)
  }

  func speakReply() {
    guard let reply = latestReply, !reply.failed else { return }
    ReplySpeaker.shared.speak(reply.text)
  }

  func stopSpeaking() {
    ReplySpeaker.shared.stop()
  }

  func leave() {
    follower?.cancel()
    follower = nil
    ReplySpeaker.shared.stop()
    if let id = conversationId {
      Task { [service] in try? await service.markThreadRead(id) }
    }
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
        guard let history = try? await self.service.history(conversationId: id, limit: 10) else { continue }
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
          try? await self.service.markThreadRead(id)
          return
        case .failed:
          self.status = .idle
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
    guard WKApplication.shared().applicationState != .active, let reply = latestReply, !reply.failed
    else { return }
    let content = UNMutableNotificationContent()
    content.title = target.agentName
    content.body = SpokenText.brief(reply.text).text
    content.sound = .default
    UNUserNotificationCenter.current().add(
      UNNotificationRequest(identifier: "reply-\(reply.id)", content: content, trigger: nil))
  }

  private func apply(_ history: ConversationHistory) {
    lastSeq = max(lastSeq, history.messages.map(\.seq).max() ?? 0)
    waitingForApproval = history.waitingForApproval
    waitingForCard = history.waitingForCard
    messages = Transcript.merge(messages, history.messages)
    if let pending = pendingQuestion,
      messages.contains(where: { $0.isFromPerson && $0.text == pending })
    {
      pendingQuestion = nil
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
      await app.refreshUnread()
    }
    .onDisappear {
      chat?.leave()
      Task { await app.refreshUnread() }
    }
  }
}

private struct ChatContent: View {
  let chat: ChatModel
  private static let bottomId = "bottom"

  var body: some View {
    ScrollViewReader { proxy in
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 10) {
          if chat.earlierCursor != nil {
            Button {
              Task { await chat.loadEarlier() }
            } label: {
              if chat.loadingEarlier {
                ProgressView()
              } else {
                Text("Earlier")
                  .font(.footnote.weight(.semibold))
              }
            }
            .buttonStyle(PillButtonStyle())
            .frame(maxWidth: .infinity)
          }
          ForEach(chat.messages) { message in
            MessageView(message: message)
              .id(message.id)
          }
          if let pending = chat.pendingQuestion {
            QuestionBubble(text: pending)
          }
          footer
          Color.clear.frame(height: 1).id(Self.bottomId)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.bottom, 56)
      }
      .onChange(of: chat.messages.last?.id) { _, _ in
        proxy.scrollTo(Self.bottomId, anchor: .bottom)
      }
      .onChange(of: chat.status) { _, _ in
        proxy.scrollTo(Self.bottomId, anchor: .bottom)
      }
      .onAppear { proxy.scrollTo(Self.bottomId, anchor: .bottom) }
    }
    .safeAreaInset(edge: .bottom, spacing: 0) {
      bottomBar
    }
    .topBar {
      // The name alone: beside the clock there is no room for an avatar too.
      Text(chat.target.agentName)
        .font(.headline)
        .lineLimit(1)
        .accessibilityAddTraits(.isHeader)
    }
  }

  /// Under the last message: the agent working, waiting on you, a problem,
  /// or (in an empty thread) a nudge to talk.
  @ViewBuilder
  private var footer: some View {
    switch chat.status {
    case .loading:
      ProgressView()
        .frame(maxWidth: .infinity)
    case .sending, .working:
      if chat.waitingForApproval {
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
          .foregroundStyle(.secondary)
      } else {
        VStack(alignment: .leading, spacing: 4) {
          TypingDots()
          Text(chat.workingLine)
            .font(.footnote.weight(.semibold))
            .lineLimit(2)
        }
      }
    case .idle:
      if chat.isEmpty {
        Text("Tap the mic to start")
          .font(.footnote)
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity)
          .padding(.top, 20)
      }
    case let .problem(text):
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
        .disabled(chat.latestReply == nil)
        .accessibilityLabel(chat.speaking ? "Stop reading" : "Read aloud")
        Spacer()
        TextFieldLink(prompt: Text("Message \(chat.target.agentName)")) {
          Image(systemName: "mic.fill")
        } onSubmit: { text in
          Task { await chat.send(text) }
        }
        .buttonStyle(CircleButtonStyle(primary: true, size: 42))
        .disabled(chat.status == .loading)
        .accessibilityLabel("Speak")
        Spacer()
        // Same width as the speaker button, so the mic stays centred.
        Color.clear
          .frame(width: 32, height: 32)
          .accessibilityHidden(true)
      }
    }
    .padding(.horizontal, 6)
    .bottomBarBackdrop()
  }
}

/// One message: the person's on the right in a bubble, the agent's in full
/// on the left, like a messages app.
private struct MessageView: View {
  let message: ChatMessage

  var body: some View {
    if message.isFromPerson {
      QuestionBubble(text: message.text)
    } else if message.failed {
      Label("Couldn't answer", systemImage: "exclamationmark.circle")
        .font(.footnote)
        .foregroundStyle(.orange)
    } else {
      Text(SpokenText.plain(message.text))
        .font(.footnote)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityLabel(SpokenText.plain(message.text))
    }
  }
}

/// What the person said, right-aligned like a sent message.
private struct QuestionBubble: View {
  let text: String

  var body: some View {
    Text(text)
      .font(.footnote)
      .padding(.horizontal, 10)
      .padding(.vertical, 6)
      .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Color.nyxSurface))
      .frame(maxWidth: .infinity, alignment: .trailing)
      .padding(.leading, 18)
  }
}
