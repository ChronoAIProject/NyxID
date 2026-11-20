import { apiClient, ApiError } from "@/lib/api-client";
import { getAssistantIdentityRevision } from "./identity";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const ACCOUNT_KEY = "nyxid:assistant-account";

export function announceAssistantAccount(userId: string | null) {
  try {
    localStorage.setItem(ACCOUNT_KEY, userId ?? "");
  } catch { /* The Assistant also checks the session when the tab becomes visible. */ }
}

/** An account hint prompts a cookie-authenticated profile read. */
export function watchAssistantSession() {
  let pending: AbortController | undefined;
  const root = document.getElementById("root");
  const hide = () => { if (root) root.hidden = true; };
  const reveal = () => { if (root) root.hidden = false; };

  async function verify(cover: boolean) {
    pending?.abort();
    const request = new AbortController();
    pending = request;
    const revision = getAssistantIdentityRevision();
    const current = () => pending === request && revision === getAssistantIdentityRevision();
    if (cover) hide();
    const timeout = window.setTimeout(() => request.abort(), 5000);
    try {
      const user = await apiClient<User>("/users/me", {
        signal: request.signal,
        preserveSessionOn401: true,
      });
      if (current() && user.id !== useAuthStore.getState().user?.id) {
        useAuthStore.getState().setUser(user);
      }
    } catch (error) {
      if (!current()) return;
      if (error instanceof ApiError && error.status === 401) {
        useAuthStore.getState().setUser(null);
      } else if (cover) {
        // The old conversation must stay covered if we cannot identify its viewer.
        // Let the normal cold-start auth flow recover, including on network failure.
        pending = undefined;
        window.location.reload();
      }
    } finally {
      window.clearTimeout(timeout);
      if (pending === request) {
        pending = undefined;
        reveal();
      }
    }
  }

  function onStorage(event: StorageEvent) {
    if (event.key === ACCOUNT_KEY && (pending || event.newValue !== (useAuthStore.getState().user?.id ?? ""))) {
      void verify(true);
    }
  }
  function onVisible() {
    if (document.visibilityState === "visible" && !pending) void verify(false);
  }
  function onPageHide(event: PageTransitionEvent) {
    if (event.persisted) {
      pending?.abort();
      pending = undefined;
      hide();
    }
  }
  function onPageShow(event: PageTransitionEvent) {
    if (event.persisted) void verify(true);
  }

  window.addEventListener("storage", onStorage);
  window.addEventListener("pagehide", onPageHide);
  window.addEventListener("pageshow", onPageShow);
  document.addEventListener("visibilitychange", onVisible);
  // Catch a switch that happened on another tab before this route mounted.
  try {
    const owner = localStorage.getItem(ACCOUNT_KEY);
    if (owner !== null && owner !== (useAuthStore.getState().user?.id ?? "")) void verify(true);
  } catch { void verify(false); }
  return () => {
    pending?.abort();
    pending = undefined;
    reveal();
    window.removeEventListener("storage", onStorage);
    window.removeEventListener("pagehide", onPageHide);
    window.removeEventListener("pageshow", onPageShow);
    document.removeEventListener("visibilitychange", onVisible);
  };
}
