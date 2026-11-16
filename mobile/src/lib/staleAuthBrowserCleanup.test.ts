import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL } from "node:url";

const require = createRequire(import.meta.url);
const {
  configureStaleAuthBrowserCleanup,
} = require("../../plugins/withStaleAuthBrowserCleanup") as {
  configureStaleAuthBrowserCleanup(contents: string): string;
};

// Fixture copied verbatim from the Expo SDK 55.0.12 generated MainActivity.kt.
const mainActivity = readFileSync(
  new URL("./__fixtures__/MainActivity.kt", import.meta.url),
  "utf8",
);
const browserManifest = readFileSync(
  new URL(
    "../../node_modules/expo-web-browser/android/src/main/AndroidManifest.xml",
    import.meta.url,
  ),
  "utf8",
);
const browserProxyActivity = readFileSync(
  new URL(
    "../../node_modules/expo-web-browser/android/src/main/java/expo/modules/webbrowser/BrowserProxyActivity.kt",
    import.meta.url,
  ),
  "utf8",
);

const startupAnchor =
  "    // @generated end expo-splashscreen\n    super.onCreate(null)";
const startup = `    // @nyxid-stale-auth-browser-cleanup
    // Process death can leave expo-web-browser's proxy task and swallow the next auth session.
    cleanupStaleAuthBrowserTasks()`;
const methods = `  private fun cleanupStaleAuthBrowserTasks() {
    if (staleAuthBrowserCleanupDone) return
    staleAuthBrowserCleanupDone = true

    getSystemService(ActivityManager::class.java)?.appTasks?.forEach { appTask ->
      try {
        val taskInfo = appTask.taskInfo
        if (
          taskInfo.baseIntent?.component?.className ==
            "expo.modules.webbrowser.BrowserProxyActivity" ||
          taskInfo.baseActivity?.className ==
            "expo.modules.webbrowser.BrowserProxyActivity"
        ) {
          appTask.finishAndRemoveTask()
        }
      } catch (_: IllegalArgumentException) {
        // The task can disappear while the list is being enumerated.
      }
    }
  }

  companion object {
    private var staleAuthBrowserCleanupDone = false
  }

`;
const methodAnchor = `  /**
   * Returns the name of the main component registered from JavaScript. This is used to schedule
   * rendering of the component.
   */`;

function expectedMainActivity() {
  return mainActivity
    .replace(
      "import expo.modules.splashscreen.SplashScreenManager",
      "import android.app.ActivityManager\nimport expo.modules.splashscreen.SplashScreenManager",
    )
    .replace(
      startupAnchor,
      `    // @generated end expo-splashscreen\n${startup}\n    super.onCreate(null)`,
    )
    .replace(methodAnchor, `${methods}${methodAnchor}`);
}

test("patches the generated Kotlin activity before super.onCreate and is idempotent", () => {
  const patched = configureStaleAuthBrowserCleanup(mainActivity);
  assert.equal(patched, expectedMainActivity());
  assert.equal(configureStaleAuthBrowserCleanup(patched), patched);

  const splashEnd = patched.indexOf("// @generated end expo-splashscreen");
  const cleanupCall = patched.indexOf("cleanupStaleAuthBrowserTasks()", splashEnd);
  const superOnCreate = patched.indexOf("super.onCreate(null)");
  assert.ok(splashEnd >= 0);
  assert.ok(cleanupCall > splashEnd);
  assert.ok(cleanupCall < superOnCreate);
  assert.match(patched, /^import android\.app\.ActivityManager$/m);
  assert.match(patched, /if \(staleAuthBrowserCleanupDone\) return/);
  assert.match(patched, /staleAuthBrowserCleanupDone = true/);
  assert.match(patched, /getSystemService\(ActivityManager::class\.java\)\?\.appTasks/);
  assert.match(patched, /taskInfo\.baseIntent\?\.component\?\.className/);
  assert.match(patched, /taskInfo\.baseActivity\?\.className/);
  assert.match(patched, /appTask\.finishAndRemoveTask\(\)/);
  assert.match(patched, /catch \(_:\s*IllegalArgumentException\)/);
});

for (const [anchor, error] of [
  [startupAnchor, /MainActivity startup anchor is missing/],
  [methodAnchor, /MainActivity method anchor is missing/],
  [/^import .+\n/gm, /MainActivity import anchor is missing/],
] as const) {
  test(`refuses to patch when the ${error.source} anchor is missing`, () => {
    assert.throws(
      () => configureStaleAuthBrowserCleanup(mainActivity.replace(anchor, "")),
      error,
    );
  });
}

test("keeps the cleanup target bound to expo-web-browser's current proxy activity", () => {
  const manifestName = browserManifest.match(
    /android:name="([^"]*BrowserProxyActivity)"/,
  )?.[1];
  const modulePackage = browserProxyActivity.match(/^package\s+([^\n]+)$/m)?.[1];
  assert.equal(manifestName, ".BrowserProxyActivity");
  assert.equal(modulePackage, "expo.modules.webbrowser");
  const fullyQualifiedName = `${modulePackage}.BrowserProxyActivity`;
  assert.equal(fullyQualifiedName, "expo.modules.webbrowser.BrowserProxyActivity");
  const patched = configureStaleAuthBrowserCleanup(mainActivity);
  assert.equal(
    patched.match(/taskInfo\.baseIntent\?\.component\?\.className ==\s*"([^"]+)"/)?.[1],
    fullyQualifiedName,
  );
  assert.equal(
    patched.match(/taskInfo\.baseActivity\?\.className ==\s*"([^"]+)"/)?.[1],
    fullyQualifiedName,
  );
  assert.doesNotMatch(patched, /realActivity/);
});
