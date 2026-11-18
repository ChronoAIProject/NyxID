import SwiftUI
import UserNotifications
import WatchKit

@main
struct NyxIDWatchApp: App {
  @WKApplicationDelegateAdaptor(WatchAppDelegate.self) private var delegate
  @State private var model = WatchAppModel.shared

  var body: some Scene {
    WindowGroup {
      RootView()
        .environment(model)
    }
  }
}

final class WatchAppDelegate: NSObject, WKApplicationDelegate, UNUserNotificationCenterDelegate {
  func applicationDidFinishLaunching() {
    PhoneLink.shared.onMessage = { message in
      WatchAppModel.shared.handle(message)
    }
    PhoneLink.shared.activate()

    let center = UNUserNotificationCenter.current()
    center.delegate = self
    center.setNotificationCategories([ApprovalNotification.category])
  }

  func applicationDidBecomeActive() {
    Task { @MainActor in await WatchAppModel.shared.refresh() }
  }

  // Show NyxID alerts while the app is open, too.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification
  ) async -> UNNotificationPresentationOptions {
    [.banner, .sound, .list]
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
  ) async {
    let userInfo = response.notification.request.content.userInfo
    await MainActor.run {
      ApprovalNotification.handle(actionIdentifier: response.actionIdentifier, userInfo: userInfo)
    }
  }
}
