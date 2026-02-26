import { Browser } from "@capacitor/browser";
import { isNative } from "./platform";

/**
 * Open a URL via system browser (native) or window.location (web).
 * Intended for OAuth flows and external links that must leave the WebView.
 */
export async function openExternal(url: string): Promise<void> {
  if (isNative) {
    await Browser.open({ url, presentationStyle: "popover" });
  } else {
    window.location.assign(url);
  }
}

/**
 * Hard-redirect to a URL — for OAuth authorization_url and similar.
 */
export function hardRedirect(url: string): void {
  if (isNative) {
    void Browser.open({ url });
  } else {
    window.location.href = url;
  }
}
