import Foundation
import Observation
import UserNotifications
import WatchKit

/// The two pages of the signed-in watch app.
enum WatchPage: Hashable {
  case unread
  case agents
}

/// Who a chat screen talks to, and optionally which thread.
struct ChatTarget: Hashable {
  let agentId: String
  let agentName: String
  var isNyxBot = false
  /// The thread to continue; nil finds the agent's home or newest thread.
  var conversationId: String?
  /// Sent as soon as the chat opens (from the Ask screen).
  var firstMessage: String?
  /// Start a new thread instead of continuing one.
  var fresh = false
}

enum WatchRoute: Hashable {
  /// The approvals waiting for a decision, starting at one of them.
  case approvals(startAt: String?)
  case feedItem(FeedItem)
  /// One agent: New chat, then its threads.
  case agent(String)
  /// "Tap to speak", addressed to an agent (NyxBot when nil). `fresh`
  /// starts a new thread; otherwise the agent's own thread continues.
  case ask(agentId: String?, fresh: Bool)
  case chat(ChatTarget)
}

extension FeedItem: Hashable {
  func hash(into hasher: inout Hasher) {
    hasher.combine(id)
  }
}

/// App-wide state: the watch's sign-in, the selected page and navigation.
@MainActor
@Observable
final class WatchAppModel {
  static let shared = WatchAppModel()

  enum Phase: Equatable {
    case loading
    case signedOut(reason: String?)
    case signingIn
    case signedIn
  }

  private(set) var phase: Phase = .loading
  var page: WatchPage = .unread
  var path: [WatchRoute] = []
  var showingSettings = false
  private(set) var agents: [AgentSummary] = []
  /// Unread NyxBot activity, newest first (empty when the feed is off).
  private(set) var unread: [FeedItem] = []
  private(set) var profile: UserProfile?
  /// The code shown while signed out; the iPhone must send it back.
  private(set) var setupCode: String?
  private var lastPairingCode: String?

  var speakReplies: Bool {
    didSet { UserDefaults.standard.set(speakReplies, forKey: Keys.speakReplies) }
  }

  /// The NyxID web app, for handing a chat to the iPhone.
  private(set) var frontendURL: URL? {
    didSet { UserDefaults.standard.set(frontendURL?.absoluteString, forKey: Keys.frontendURL) }
  }

  let api: WatchAPI
  var service: WatchService { WatchService(api: api) }
  var nyxBot: AgentSummary? { agents.first(where: \.isNyxBot) }

  private enum Keys {
    static let speakReplies = "speakReplies"
    static let frontendURL = "frontendURL"
  }

  init(api: WatchAPI = WatchAPI(storage: KeychainSessionStorage())) {
    self.api = api
    self.speakReplies = UserDefaults.standard.object(forKey: Keys.speakReplies) as? Bool ?? true
    self.frontendURL = UserDefaults.standard.string(forKey: Keys.frontendURL).flatMap(URL.init(string:))
    Task { await bootstrap() }
  }

  private func bootstrap() async {
    await api.onSignedOut { [weak self] in
      Task { @MainActor in self?.didSignOut(reason: "Signed out") }
    }
    if await api.isSignedIn {
      phase = .signedIn
      PhoneLink.shared.sayHello(code: nil)
      await Self.askForAlerts()
      if let route = launchRoute {
        launchRoute = nil
        page = .unread
        path = route
      }
      await refresh()
    } else if phase == .loading {
      showSetup(reason: nil)
      launchRoute = nil
      queuedDecision = nil
    }
  }

  /// Signed out: show a fresh setup code and tell the iPhone about it.
  private func showSetup(reason: String?) {
    let code = SetupCode.generate()
    setupCode = code
    phase = .signedOut(reason: reason)
    PhoneLink.shared.sayHello(code: code)
  }

  // MARK: iPhone link

  func handle(_ message: PhoneMessage) {
    switch message {
    case let .pair(code, apiBaseURL, match, frontend, expiresAt):
      // Only the code this watch is showing right now can sign it in. The
      // same message can also arrive twice (direct, then queued).
      guard case .signedOut = phase, SetupCode.accepts(match, showing: setupCode),
        code != lastPairingCode
      else { return }
      lastPairingCode = code
      if let expiresAt, expiresAt < Date() {
        showSetup(reason: "Code expired")
        return
      }
      phase = .signingIn
      Task {
        do {
          try await api.redeem(code: code, apiBaseURL: apiBaseURL, context: WatchClientInfo.loginContext())
          frontendURL = frontend
          setupCode = nil
          phase = .signedIn
          page = .unread
          path = []
          PhoneLink.shared.sayHello(code: nil)
          WKInterfaceDevice.current().play(.success)
          await Self.askForAlerts()
          await refresh()
        } catch let error as WatchAPIError {
          showSetup(reason: Self.setupFailure(error))
          WKInterfaceDevice.current().play(.failure)
        } catch {
          showSetup(reason: "Try again")
        }
      }
    case .signOut:
      Task { await signOut(reason: "Signed out from iPhone") }
    }
  }

  /// Lets NyxID alerts (with Approve/Deny on approvals) show on the watch
  /// while the app is open. Asked once, after the watch is signed in.
  static func askForAlerts() async {
    let center = UNUserNotificationCenter.current()
    guard await center.notificationSettings().authorizationStatus == .notDetermined else { return }
    _ = try? await center.requestAuthorization(options: [.alert, .sound])
  }

  static func setupFailure(_ error: WatchAPIError) -> String {
    switch error.errorKey {
    case "login_code_expired": return "Code expired"
    case "login_code_redeemed", "login_code_cancelled", "login_code_invalid": return "Code already used"
    case "mfa_step_up_required": return "Verify with MFA on iPhone"
    case "login_code_rate_limited": return "Too many tries. Wait a bit"
    default: return error.displayMessage
    }
  }

  // MARK: Session

  func signOut(reason: String? = nil) async {
    await api.signOut()
    didSignOut(reason: reason)
  }

  private func didSignOut(reason: String?) {
    path = []
    page = .unread
    showingSettings = false
    agents = []
    unread = []
    profile = nil
    ReplySpeaker.shared.stop()
    showSetup(reason: reason)
  }

  func refresh() async {
    guard phase == .signedIn else { return }
    async let agentList = try? service.agents()
    async let account = try? service.profile()
    let (list, me) = await (agentList, account)
    if let list { agents = list }
    if let me { profile = me }
    await refreshUnread()
  }

  /// Reloads unread activity; returns false when it couldn't be loaded.
  @discardableResult
  func refreshUnread() async -> Bool {
    guard phase == .signedIn else { return false }
    do {
      unread = try await service.feed(limit: 50, unreadOnly: true)?.items ?? []
      return true
    } catch {
      return false
    }
  }

  /// Threads with unread activity.
  var unreadThreadIds: Set<String> {
    Set(unread.compactMap { $0.source?.conversationId })
  }

  /// Agents with unread activity in any of their threads.
  var unreadAgentIds: Set<String> {
    Set(unread.compactMap { item in
      if item.actor?.type == "agent", let id = item.actor?.id { return id }
      return item.source?.agentId
    })
  }

  func agent(id: String?) -> AgentSummary? {
    guard let id else { return nil }
    return agents.first { $0.id == id }
  }

  // MARK: Asking

  /// Opens "Tap to speak" for an agent (NyxBot when nil). `fresh` starts
  /// a new thread; the header mic continues the agent's own thread.
  func ask(agentId: String?, fresh: Bool = false) {
    path.append(.ask(agentId: agentId, fresh: fresh))
  }

  /// Leaves the Ask screen for the chat its question went to.
  func openChat(_ target: ChatTarget) {
    if case .ask = path.last { path.removeLast() }
    path.append(.chat(target))
  }

  func chatTarget(for agent: AgentSummary, firstMessage: String? = nil, fresh: Bool = false) -> ChatTarget {
    ChatTarget(
      agentId: agent.id, agentName: agent.label, isNyxBot: agent.isNyxBot,
      conversationId: fresh ? nil : agent.homeConversationId, firstMessage: firstMessage, fresh: fresh)
  }

  // MARK: Notification routing

  /// A decision chosen on a notification, made once its approval opens.
  private(set) var queuedDecision: QueuedDecision?

  func queueDecision(id: String, approve: Bool) {
    queuedDecision = QueuedDecision(id: id, approve: approve)
  }

  func takeQueuedDecision(for id: String) -> QueuedDecision? {
    guard let queued = queuedDecision, queued.id == id else { return nil }
    queuedDecision = nil
    return queued
  }

  /// Opens an approval from a notification.
  func openApproval(_ id: String) {
    open([.approvals(startAt: id)])
  }

  func openUnread() {
    open([])
  }

  /// A notification opened before the session finished loading.
  private var launchRoute: [WatchRoute]?

  private func open(_ route: [WatchRoute]) {
    switch phase {
    case .signedIn:
      page = .unread
      path = route
    case .loading:
      launchRoute = route
    case .signedOut, .signingIn:
      queuedDecision = nil
    }
  }
}
