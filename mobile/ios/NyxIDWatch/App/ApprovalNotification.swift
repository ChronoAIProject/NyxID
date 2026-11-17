import Foundation
import UserNotifications

/// Approve/Deny on approval alerts. The backend tags only decidable service
/// approvals with this category (`push_service::apns_alert_category`).
/// Both actions open the watch app, which makes the decision with its own
/// session and shows the result; nothing is decided silently.
enum ApprovalNotification {
  static let categoryIdentifier = "APPROVAL_REQUEST"
  static let approveIdentifier = "APPROVE"
  static let denyIdentifier = "DENY"

  static var category: UNNotificationCategory {
    UNNotificationCategory(
      identifier: categoryIdentifier,
      actions: [
        UNNotificationAction(
          identifier: approveIdentifier, title: "Approve",
          options: [.foreground, .authenticationRequired]),
        UNNotificationAction(
          identifier: denyIdentifier, title: "Deny",
          options: [.foreground, .destructive]),
      ],
      intentIdentifiers: [])
  }

  @MainActor
  static func handle(actionIdentifier: String, userInfo: [AnyHashable: Any]) {
    let model = WatchAppModel.shared
    let type = userInfo["type"] as? String
    if type == "approval_request",
      let id = (userInfo["request_id"] as? String) ?? (userInfo["challenge_id"] as? String),
      (try? PathSegment(id)) != nil
    {
      switch actionIdentifier {
      case approveIdentifier: model.queueDecision(id: id, approve: true)
      case denyIdentifier: model.queueDecision(id: id, approve: false)
      default: break
      }
      model.openApproval(id)
      return
    }
    if userInfo["notification_id"] != nil {
      model.openUnread()
    }
  }
}

extension WatchAppModel {
  struct QueuedDecision: Equatable {
    let id: String
    let approve: Bool
  }
}
