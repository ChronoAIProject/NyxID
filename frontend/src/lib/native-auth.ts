import { App, type URLOpenListenerEvent } from "@capacitor/app";
import { Capacitor } from "@capacitor/core";
import { API_ORIGIN } from "./urls";

const API_BASE = `${API_ORIGIN}/api/v1`;

/**
 * Inject tokens received from native OAuth deep link as HTTP-only cookies
 * by calling a backend endpoint, then trigger auth state refresh.
 */
async function injectTokens(params: URLSearchParams): Promise<boolean> {
  const session = params.get("session_token");
  const access = params.get("access_token");
  const refresh = params.get("refresh_token");

  if (!session || !access || !refresh) return false;

  const res = await fetch(`${API_BASE}/auth/native-token-exchange`, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      session_token: session,
      access_token: access,
      refresh_token: refresh,
    }),
  });

  return res.ok;
}

/**
 * Register the deep-link listener once during app startup.
 * When the backend redirects to nyxid://auth/callback?..., iOS opens the app
 * and this listener fires.
 */
export function registerDeepLinkHandler(onAuthSuccess: () => void): void {
  if (!Capacitor.isNativePlatform()) return;

  void App.addListener("appUrlOpen", (event: URLOpenListenerEvent) => {
    const url = new URL(event.url);

    if (url.host !== "auth" || url.pathname !== "/callback") return;

    const error = url.searchParams.get("error");
    if (error) {
      console.error("[native-auth] OAuth error:", error);
      return;
    }

    void injectTokens(url.searchParams).then((ok) => {
      if (ok) onAuthSuccess();
    });
  });
}
