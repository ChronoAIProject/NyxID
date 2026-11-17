import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { URL } from "node:url";
import test from "node:test";

const require = createRequire(import.meta.url);
const { configureSceneLifecycle, configureSceneManifest, SCENE_MANIFEST } =
  require("../../plugins/withSceneLifecycle") as {
    configureSceneLifecycle(source: string): string;
    configureSceneManifest(
      plist: Record<string, unknown>,
    ): Record<string, unknown>;
    SCENE_MANIFEST: Record<string, unknown>;
  };
const appDelegatePath = new URL(
  "../../ios/NyxIDMobile/AppDelegate.swift",
  import.meta.url,
);
const infoPlistPath = new URL(
  "../../ios/NyxIDMobile/Info.plist",
  import.meta.url,
);
const appDelegate = readFileSync(appDelegatePath, "utf8");

// The Expo 55 / React Native 0.83 template body that prebuild starts from.
const LEGACY_WINDOW = `#if os(iOS) || os(tvOS)
    window = UIWindow(frame: UIScreen.main.bounds)
    factory.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: launchOptions)
#endif
`;

function templateFrom(generated: string): string {
  return generated
    .replace(
      "    // The window and React root are created per scene in SceneDelegate.\n",
      LEGACY_WINDOW,
    )
    .replace(
      /\n\/\/\/ Owns the app window\.[^]*?\n}\n(?=\nclass ReactNativeDelegate)/,
      "",
    );
}

test("Expo regeneration moves the window into a scene delegate and is idempotent", () => {
  const template = templateFrom(appDelegate);
  assert.match(template, /window = UIWindow\(frame: UIScreen\.main\.bounds\)/);
  assert.doesNotMatch(template, /SceneDelegate/);
  const generated = configureSceneLifecycle(template);
  assert.equal(generated, appDelegate);
  assert.equal(configureSceneLifecycle(generated), generated);
});

test("the app delegate no longer creates a window; the scene delegate starts React once", () => {
  const appDelegateClass = appDelegate.slice(
    0,
    appDelegate.indexOf("class SceneDelegate"),
  );
  assert.doesNotMatch(appDelegateClass, /UIWindow\(|startReactNative/);
  assert.equal(appDelegate.match(/factory\.startReactNative\(/g)?.length, 1);
  assert.match(appDelegate, /UIWindow\(windowScene: windowScene\)/);
  // Cold-start links must reach Linking.getInitialURL and the dev launcher.
  assert.match(appDelegate, /launchOptions\[\.url\] = context\.url/);
  assert.match(
    appDelegate,
    /"UIApplicationLaunchOptionsUserActivityKey": activity/,
  );
  assert.match(appDelegate, /func scene\(_ scene: UIScene, openURLContexts/);
  assert.match(
    appDelegate,
    /func scene\(_ scene: UIScene, continue userActivity/,
  );
});

test("native generation refuses to silently skip scene adoption after a template change", () => {
  assert.throws(
    () =>
      configureSceneLifecycle(
        "class ReactNativeDelegate: ExpoReactNativeFactoryDelegate {\n}\n",
      ),
    /window anchor is missing/,
  );
  assert.throws(
    () => configureSceneLifecycle(LEGACY_WINDOW),
    /ReactNativeDelegate anchor is missing/,
  );
});

test("Info.plist declares the single-window scene the delegate class implements", () => {
  const plist = readFileSync(infoPlistPath, "utf8");
  const start = plist.indexOf("<key>UIApplicationSceneManifest</key>");
  const manifest =
    start < 0
      ? undefined
      : plist.slice(
          start,
          plist.indexOf("<key>UIBackgroundModes</key>", start),
        );
  assert.ok(manifest, "UIApplicationSceneManifest is missing from Info.plist");
  assert.match(
    manifest,
    /<key>UIApplicationSupportsMultipleScenes<\/key>\s*<false\/>/,
  );
  assert.match(manifest, /<key>UIWindowSceneSessionRoleApplication<\/key>/);
  assert.match(
    manifest,
    /<key>UISceneDelegateClassName<\/key>\s*<string>\$\(PRODUCT_MODULE_NAME\)\.SceneDelegate<\/string>/,
  );
  // Prebuild writes the same manifest the checked-in plist carries.
  assert.deepEqual(configureSceneManifest({}), {
    UIApplicationSceneManifest: SCENE_MANIFEST,
  });
  const roles = (
    SCENE_MANIFEST.UISceneConfigurations as Record<
      string,
      Array<Record<string, string>>
    >
  ).UIWindowSceneSessionRoleApplication;
  assert.ok(roles);
  assert.equal(
    roles[0]!.UISceneDelegateClassName,
    "$(PRODUCT_MODULE_NAME).SceneDelegate",
  );
  assert.equal(SCENE_MANIFEST.UIApplicationSupportsMultipleScenes, false);
});
