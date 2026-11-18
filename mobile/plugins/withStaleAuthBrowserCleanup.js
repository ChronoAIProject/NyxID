const { withMainActivity } = require("expo/config-plugins");

const IMPORT = "import android.app.ActivityManager";
const MARKER = "    // @nyxid-stale-auth-browser-cleanup";
const SPLASH_END = "    // @generated end expo-splashscreen";
const SUPER_ANCHOR = "    super.onCreate(null)";
const STARTUP_ANCHOR = `${SPLASH_END}\n${SUPER_ANCHOR}`;
const METHOD_ANCHOR = `  /**
   * Returns the name of the main component registered from JavaScript. This is used to schedule
   * rendering of the component.
   */`;
const STARTUP = `${MARKER}
    // Process death can leave expo-web-browser's proxy task and swallow the next auth session.
    cleanupStaleAuthBrowserTasks()`;
const METHODS = `  private fun cleanupStaleAuthBrowserTasks() {
    if (staleAuthBrowserCleanupDone) return
    staleAuthBrowserCleanupDone = true

    try {
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
    } catch (_: RuntimeException) {
      // Best-effort cleanup must never prevent app startup.
    }
  }

  companion object {
    private var staleAuthBrowserCleanupDone = false
  }

`;

function configureStaleAuthBrowserCleanup(contents) {
  if (contents.includes(MARKER)) return contents;
  if (!contents.includes(STARTUP_ANCHOR)) {
    throw new Error(
      "Stale auth browser cleanup: MainActivity startup anchor is missing.",
    );
  }
  if (!contents.includes(METHOD_ANCHOR)) {
    throw new Error(
      "Stale auth browser cleanup: MainActivity method anchor is missing.",
    );
  }
  if (!contents.includes(IMPORT)) {
    const firstImport = contents.match(/^import .+$/m)?.[0];
    if (!firstImport) {
      throw new Error(
        "Stale auth browser cleanup: MainActivity import anchor is missing.",
      );
    }
    contents = contents.replace(firstImport, `${IMPORT}\n${firstImport}`);
  }
  contents = contents.replace(
    STARTUP_ANCHOR,
    `${SPLASH_END}\n${STARTUP}\n${SUPER_ANCHOR}`,
  );
  return contents.replace(METHOD_ANCHOR, `${METHODS}${METHOD_ANCHOR}`);
}

module.exports = (config) =>
  withMainActivity(config, (config) => {
    if (config.modResults.language !== "kt") {
      throw new Error(
        "Stale auth browser cleanup requires the Kotlin MainActivity.",
      );
    }
    config.modResults.contents = configureStaleAuthBrowserCleanup(
      config.modResults.contents,
    );
    return config;
  });
module.exports.configureStaleAuthBrowserCleanup =
  configureStaleAuthBrowserCleanup;
