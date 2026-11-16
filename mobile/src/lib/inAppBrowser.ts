import * as WebBrowser from "expo-web-browser";
import { Platform } from "react-native";
import type {
  AuthSessionOpenOptions,
  WebBrowserOpenOptions,
} from "expo-web-browser";

let browserPackagePromise: Promise<string | undefined> | undefined;

async function resolveBrowserPackage(): Promise<string | undefined> {
  try {
    const browsers = await WebBrowser.getCustomTabsSupportingBrowsersAsync();
    return (
      browsers?.preferredBrowserPackage ??
      browsers?.defaultBrowserPackage ??
      browsers?.browserPackages?.[0]
    );
  } catch {
    return undefined;
  }
}

/** Add the selected Android Custom Tabs package while preserving caller options. */
export async function inAppBrowserOptions(
  extra?: AuthSessionOpenOptions,
): Promise<AuthSessionOpenOptions | undefined> {
  if (Platform?.OS !== "android") return extra;
  browserPackagePromise ??= resolveBrowserPackage();
  const browserPackage = await browserPackagePromise;
  if (!browserPackage) return extra;
  return { ...extra, browserPackage };
}

/** Explicit Android browsers keep verified App Links from capturing web opens. */
export function openInAppBrowser(url: string, extra?: WebBrowserOpenOptions) {
  const open = (options?: WebBrowserOpenOptions) =>
    options === undefined
      ? WebBrowser.openBrowserAsync(url)
      : WebBrowser.openBrowserAsync(url, options);
  if (Platform?.OS !== "android") return open(extra);
  return inAppBrowserOptions(extra).then(open);
}
