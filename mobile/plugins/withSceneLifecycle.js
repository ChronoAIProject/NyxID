const { withAppDelegate, withInfoPlist } = require("expo/config-plugins");

// Apps built with the iOS 27 SDK must adopt the UIScene lifecycle: UIKit
// terminates a non-scene app at launch (EXC_BREAKPOINT in
// _UIApplicationEvaluateRuntimeIssueForNoSceneLifecycleAdoption). Expo 55 and
// React Native 0.83 templates still create the window in the app delegate, so
// the window and React root move to a scene delegate here.

const SCENE_DELEGATE_CLASS = "$(PRODUCT_MODULE_NAME).SceneDelegate";

const SCENE_MANIFEST = {
  UIApplicationSupportsMultipleScenes: false,
  UISceneConfigurations: {
    UIWindowSceneSessionRoleApplication: [
      {
        UISceneConfigurationName: "Default Configuration",
        UISceneDelegateClassName: SCENE_DELEGATE_CLASS,
      },
    ],
  },
};

const LEGACY_WINDOW =
  /#if os\(iOS\) \|\| os\(tvOS\)\n {4}window = UIWindow\(frame: UIScreen\.main\.bounds\)\n {4}factory\.startReactNative\(\n {6}withModuleName: "main",\n {6}in: window,\n {6}launchOptions: launchOptions\)\n#endif\n/;

const SCENE_NOTE = `    // The window and React root are created per scene in SceneDelegate.
`;

const SCENE_DELEGATE = `
/// Owns the app window. UIKit hands URLs and user activities to the scene
/// instead of the app delegate; forward them so Expo subscribers (including
/// the dev launcher) and React Native Linking keep a single entry point.
class SceneDelegate: UIResponder, UIWindowSceneDelegate {
  var window: UIWindow?

  func scene(
    _ scene: UIScene,
    willConnectTo session: UISceneSession,
    options connectionOptions: UIScene.ConnectionOptions
  ) {
    guard
      let windowScene = scene as? UIWindowScene,
      let appDelegate = UIApplication.shared.delegate as? AppDelegate,
      let factory = appDelegate.reactNativeFactory
    else { return }

    let window = UIWindow(windowScene: windowScene)
    self.window = window
    appDelegate.window = window
    // A cold-start link arrives in the connection options rather than the
    // app's launch options; Linking.getInitialURL and the dev launcher read it
    // from the launch options given to React Native.
    factory.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: Self.launchOptions(from: connectionOptions))
  }

  func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
    guard let appDelegate = UIApplication.shared.delegate as? AppDelegate else { return }
    for context in URLContexts {
      var options: [UIApplication.OpenURLOptionsKey: Any] = [
        .openInPlace: context.options.openInPlace
      ]
      if let source = context.options.sourceApplication { options[.sourceApplication] = source }
      if let annotation = context.options.annotation { options[.annotation] = annotation }
      _ = appDelegate.application(UIApplication.shared, open: context.url, options: options)
    }
  }

  func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
    guard let appDelegate = UIApplication.shared.delegate as? AppDelegate else { return }
    _ = appDelegate.application(
      UIApplication.shared, continue: userActivity, restorationHandler: { _ in })
  }

  static func launchOptions(
    from connectionOptions: UIScene.ConnectionOptions
  ) -> [UIApplication.LaunchOptionsKey: Any]? {
    var launchOptions: [UIApplication.LaunchOptionsKey: Any] = [:]
    if let context = connectionOptions.urlContexts.first {
      launchOptions[.url] = context.url
      if let source = context.options.sourceApplication { launchOptions[.sourceApplication] = source }
    }
    if let activity = connectionOptions.userActivities.first(where: {
      $0.activityType == NSUserActivityTypeBrowsingWeb
    }) {
      launchOptions[.userActivityDictionary] = [
        "UIApplicationLaunchOptionsUserActivityKey": activity,
        UIApplication.LaunchOptionsKey.userActivityType.rawValue: activity.activityType,
      ]
    }
    return launchOptions.isEmpty ? nil : launchOptions
  }
}
`;

function configureSceneLifecycle(contents) {
  if (LEGACY_WINDOW.test(contents)) {
    contents = contents.replace(LEGACY_WINDOW, SCENE_NOTE);
  } else if (!contents.includes(SCENE_NOTE)) {
    throw new Error(
      "Scene lifecycle: AppDelegate window anchor is missing; update plugins/withSceneLifecycle.js for the new template.",
    );
  }
  if (!/^class SceneDelegate: UIResponder, UIWindowSceneDelegate \{$/m.test(contents)) {
    const anchor = "\nclass ReactNativeDelegate: ExpoReactNativeFactoryDelegate {";
    if (!contents.includes(anchor))
      throw new Error(
        "Scene lifecycle: ReactNativeDelegate anchor is missing; update plugins/withSceneLifecycle.js for the new template.",
      );
    contents = contents.replace(anchor, `${SCENE_DELEGATE}${anchor}`);
  }
  return contents;
}

function configureSceneManifest(infoPlist) {
  return { ...infoPlist, UIApplicationSceneManifest: SCENE_MANIFEST };
}

module.exports = (config) => {
  config = withInfoPlist(config, (config) => {
    config.modResults = configureSceneManifest(config.modResults);
    return config;
  });
  return withAppDelegate(config, (config) => {
    if (config.modResults.language !== "swift")
      throw new Error("Scene lifecycle requires the Swift AppDelegate.");
    config.modResults.contents = configureSceneLifecycle(
      config.modResults.contents,
    );
    return config;
  });
};
module.exports.configureSceneLifecycle = configureSceneLifecycle;
module.exports.configureSceneManifest = configureSceneManifest;
module.exports.SCENE_MANIFEST = SCENE_MANIFEST;
