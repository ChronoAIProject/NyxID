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

async function getBrowserPackage(): Promise<string | undefined> {
  const pending = (browserPackagePromise ??= resolveBrowserPackage());
  const browserPackage = await pending;
  if (!browserPackage && browserPackagePromise === pending)
    browserPackagePromise = undefined;
  return browserPackage;
}

function clearBrowserPackageCache() {
  browserPackagePromise = undefined;
}

function withoutBrowserPackage(options?: WebBrowserOpenOptions) {
  if (!options?.browserPackage) return options;
  const { browserPackage: _browserPackage, ...rest } = options;
  return rest;
}

function openBrowser(url: string, options?: WebBrowserOpenOptions) {
  return options === undefined
    ? WebBrowser.openBrowserAsync(url)
    : WebBrowser.openBrowserAsync(url, options);
}

function openAuthSession(
  url: string,
  returnUrl: string,
  options?: AuthSessionOpenOptions,
) {
  return options === undefined
    ? WebBrowser.openAuthSessionAsync(url, returnUrl)
    : WebBrowser.openAuthSessionAsync(url, returnUrl, options);
}

/** Add the selected Android Custom Tabs package while preserving caller options. */
export async function inAppBrowserOptions(
  extra?: AuthSessionOpenOptions,
): Promise<AuthSessionOpenOptions | undefined> {
  if (Platform?.OS !== "android") return extra;
  const browserPackage = await getBrowserPackage();
  if (!browserPackage) return extra;
  return { ...extra, browserPackage };
}

/** Explicit Android browsers keep verified App Links from capturing web opens. */
export async function openInAppBrowser(
  url: string,
  extra?: WebBrowserOpenOptions,
) {
  if (Platform?.OS !== "android") return openBrowser(url, extra);
  const options = await inAppBrowserOptions(extra);
  try {
    return await openBrowser(url, options);
  } catch (error) {
    if (!options?.browserPackage) throw error;
    clearBrowserPackageCache();
    return openBrowser(url, withoutBrowserPackage(extra));
  }
}

/** Open an auth session in an explicit Android browser with one fallback retry. */
export async function openAuthSessionInAppBrowser(
  url: string,
  returnUrl: string,
  extra?: AuthSessionOpenOptions,
) {
  const options = await inAppBrowserOptions(extra);
  try {
    return await openAuthSession(url, returnUrl, options);
  } catch (error) {
    if (Platform?.OS !== "android" || !options?.browserPackage) throw error;
    clearBrowserPackageCache();
    return openAuthSession(url, returnUrl, withoutBrowserPackage(extra));
  }
}
