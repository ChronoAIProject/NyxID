internal import Expo
import React
import ReactAppDependencyProvider

#if DEBUG && targetEnvironment(simulator)
import ObjectiveC
import WebKit
#endif

@main
class AppDelegate: ExpoAppDelegate {
  var window: UIWindow?

  var reactNativeDelegate: ExpoReactNativeFactoryDelegate?
  var reactNativeFactory: RCTReactNativeFactory?

  public override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
#if DEBUG && targetEnvironment(simulator)
    SimulatorVoiceCapture.install()
#endif

    let delegate = ReactNativeDelegate()
    let factory = ExpoReactNativeFactory(delegate: delegate)
    delegate.dependencyProvider = RCTAppDependencyProvider()

    reactNativeDelegate = delegate
    reactNativeFactory = factory

#if os(iOS) || os(tvOS)
    window = UIWindow(frame: UIScreen.main.bounds)
    factory.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: launchOptions)
#endif

    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  // Linking API
  public override func application(
    _ app: UIApplication,
    open url: URL,
    options: [UIApplication.OpenURLOptionsKey: Any] = [:]
  ) -> Bool {
    return super.application(app, open: url, options: options) || RCTLinkingManager.application(app, open: url, options: options)
  }

  // Universal Links
  public override func application(
    _ application: UIApplication,
    continue userActivity: NSUserActivity,
    restorationHandler: @escaping ([UIUserActivityRestoring]?) -> Void
  ) -> Bool {
    let result = RCTLinkingManager.application(application, continue: userActivity, restorationHandler: restorationHandler)
    return super.application(application, continue: userActivity, restorationHandler: restorationHandler) || result
  }
}

#if DEBUG && targetEnvironment(simulator)
// Opt-in diagnostic path: real Simulator capture depends on host/runtime audio
// support and can stall. Leave WebKit's mock capture enabled by default so the
// voice mock-device guard can reject it. Device and Release builds omit this SPI.
private enum SimulatorVoiceCapture {
  private typealias ObjectGetter = @convention(c) (AnyObject, Selector) -> Unmanaged<AnyObject>?
  private typealias ConfigurationGetter = @convention(c) (AnyObject, Selector) -> Unmanaged<AnyObject>
  private typealias BooleanSetter = @convention(c) (AnyObject, Selector, Bool) -> Void

  static func install() {
    guard ProcessInfo.processInfo.environment["NYXID_SIMULATOR_REAL_MICROPHONE"] == "1" else {
      return
    }
    _ = installOnce
  }

  private static let installOnce: Void = {
    let configurationSelector = NSSelectorFromString("setUpWkWebViewConfig")
    guard let webViewClass = NSClassFromString("RNCWebViewImpl"),
      let configurationMethod = class_getInstanceMethod(webViewClass, configurationSelector)
    else {
      return
    }

    let original = unsafeBitCast(
      method_getImplementation(configurationMethod), to: ConfigurationGetter.self)
    let sourceSelector = NSSelectorFromString("source")
    let mockCaptureSelector = NSSelectorFromString("_setMockCaptureDevicesEnabled:")

    let replacement: @convention(block) (AnyObject) -> AnyObject = { view in
      let result = original(view, configurationSelector).takeUnretainedValue()
      guard let configuration = result as? WKWebViewConfiguration,
        let receiver = view as? NSObject,
        receiver.responds(to: sourceSelector),
        let sourceImplementation = receiver.method(for: sourceSelector)
      else {
        return result
      }

      let getSource = unsafeBitCast(sourceImplementation, to: ObjectGetter.self)
      guard let source = getSource(receiver, sourceSelector)?.takeUnretainedValue() as? NSDictionary,
        let html = source["html"] as? String,
        html.contains("window.__nyxVoiceMessage")
      else {
        return result
      }

      let preferences = configuration.preferences
      guard preferences.responds(to: mockCaptureSelector),
        let setterImplementation = preferences.method(for: mockCaptureSelector)
      else {
        return result
      }

      let setMockCapture = unsafeBitCast(setterImplementation, to: BooleanSetter.self)
      setMockCapture(preferences, mockCaptureSelector, false)
      return result
    }

    method_setImplementation(configurationMethod, imp_implementationWithBlock(replacement))
  }()
}
#endif

class ReactNativeDelegate: ExpoReactNativeFactoryDelegate {
  // Extension point for config-plugins

  override func sourceURL(for bridge: RCTBridge) -> URL? {
    // needed to return the correct URL for expo-dev-client.
    bridge.bundleURL ?? bundleURL()
  }

  override func bundleURL() -> URL? {
#if DEBUG
    return RCTBundleURLProvider.sharedSettings().jsBundleURL(forBundleRoot: ".expo/.virtual-metro-entry")
#else
    return Bundle.main.url(forResource: "main", withExtension: "jsbundle")
#endif
  }
}
