import { useEffect, useRef, useState } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { router } from "@/router";
import { startBuildUpdates } from "@/lib/build-updates";
import { useBuildUpdateStore, UPDATE_REMINDER_DELAY } from "@/stores/build-update-store";
import { BUILD_UPDATE_VIEW_SWITCH } from "@/lib/build-update-navigation";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";

function readOnlyPage() {
  const { pathname, search, hash } = window.location;
  if (search || hash) return false;
  return pathname === "/" || pathname === "/dashboard" ||
    pathname === "/privacy" || pathname === "/terms" ||
    pathname === "/docs" || pathname.startsWith("/docs/") ||
    pathname === "/blog" || pathname.startsWith("/blog/");
}

export function useBuildUpdates(ready: boolean, queryClient: QueryClient) {
  const pending = useBuildUpdateStore((state) => state.pending);
  const [now, setNow] = useState(Date.now);
  const assistantBuild = document.querySelector('meta[name="nyxid-assistant-build-id"]')?.getAttribute("content");
  const assistantChanged = !pending?.build.assistant || !assistantBuild || pending.build.assistant !== assistantBuild;
  const available = ready && import.meta.env.PROD && !!pending && pending.build.buildId !== __BUILD_ID__ && now - pending.detectedAt >= UPDATE_REMINDER_DELAY;
  const updatesRef = useRef<ReturnType<typeof startBuildUpdates> | null>(null);
  useEffect(() => {
    if (!pending) return;
    const timer = setTimeout(() => setNow(Date.now()), Math.max(0, pending.detectedAt + UPDATE_REMINDER_DELAY - Date.now()));
    return () => clearTimeout(timer);
  }, [pending]);
  useEffect(() => {
    if (!import.meta.env.PROD || !ready) return;
    // An untouched reading tab has no user-created ephemeral state. Once used,
    // keep its state until the person deliberately refreshes, even after saving.
    let untouched = readOnlyPage();
    if (useBuildUpdateStore.getState().pending?.build.buildId === __BUILD_ID__) {
      useBuildUpdateStore.getState().clear();
    }
    let hiddenAt = document.hidden ? Date.now() : Infinity;
    const markUsed = () => { untouched = false; };
    const trackVisibility = () => { hiddenAt = document.hidden ? Date.now() : Infinity; };
    const unsubscribe = router.subscribe("onBeforeNavigate", () => { untouched = false; });
    const events = ["pointerdown", "keydown", "input", "change", "submit", "drop"] as const;
    for (const event of events) document.addEventListener(event, markUsed, true);
    document.addEventListener("visibilitychange", trackVisibility);

    const updates = startBuildUpdates({
      pendingBuild: useBuildUpdateStore.getState().pending?.build,
      canAutoReload: () => untouched && readOnlyPage() && document.hidden &&
        Date.now() - hiddenAt >= 60_000 && queryClient.isMutating() === 0 &&
        !document.querySelector('[role="dialog"], [role="alertdialog"], textarea, [contenteditable="true"]') &&
        ![...document.querySelectorAll("input")].some((input) =>
          input.type !== "search" || input.value !== ""),
      onReady: (build) => {
        if (build) useBuildUpdateStore.getState().record(build);
      },
      onObserved: (build) => {
        if (!build || build.buildId === __BUILD_ID__) useBuildUpdateStore.getState().clear();
      },
    });
    const applyOnSwitch = () => {
      const safeView = readOnlyPage() || ["/assistant", "/assistant/plugins", "/assistant/approvals", "/assistant/automations", "/assistant/machines"].includes(window.location.pathname);
      if (!safeView) return;
      const build = useBuildUpdateStore.getState().pending?.build;
      if (!build || queryClient.isMutating() > 0 || document.hidden) return;
      if (build.assistant && assistantBuild && build.assistant === assistantBuild) return;
      const destination = window.location.href;
      const canApply = () => {
        const localTurnActive = directAssistantTransport.getConversationsSnapshot().some((conversation) => {
          const status = directAssistantTransport.getHistorySnapshot(conversation.id)?.activeTurn?.status;
          return status === "running" || status === "waiting";
        });
        return !localTurnActive && queryClient.isMutating() === 0 && !document.hidden &&
          window.location.href === destination &&
          !document.querySelector('[role="dialog"], [role="alertdialog"]') &&
          ![...document.querySelectorAll("input, textarea, [contenteditable='true']")].some((element) =>
            element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement
              ? element.value.trim() !== "" : !!element.textContent?.trim());
      };
      if (canApply()) void updates.apply(canApply);
    };
    // Ordinary router links also reset views. Programmatic redirects and search
    // changes only participate when explicitly marked by chat navigation.
    let linkDestination: string | null = null;
    const trackLink = (event: MouseEvent) => {
      linkDestination = null;
      const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
      if (!(anchor instanceof HTMLAnchorElement) || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || anchor.target || anchor.hasAttribute("download")) return;
      const url = new URL(anchor.href);
      if (url.origin === window.location.origin && url.pathname !== window.location.pathname) {
        linkDestination = url.pathname;
      }
    };
    const unsubscribeResolved = router.subscribe("onResolved", () => {
      const destination = linkDestination;
      linkDestination = null;
      if (destination === window.location.pathname) applyOnSwitch();
    });
    document.addEventListener("click", trackLink, true);
    window.addEventListener(BUILD_UPDATE_VIEW_SWITCH, applyOnSwitch);
    updatesRef.current = updates;
    return () => {
      updates.stop();
      updatesRef.current = null;
      unsubscribe();
      unsubscribeResolved();
      document.removeEventListener("click", trackLink, true);
      window.removeEventListener(BUILD_UPDATE_VIEW_SWITCH, applyOnSwitch);
      for (const event of events) document.removeEventListener(event, markUsed, true);
      document.removeEventListener("visibilitychange", trackVisibility);
    };
  }, [ready, queryClient, assistantBuild]);
  return { available, assistantChanged, refresh: () => updatesRef.current?.apply() ?? Promise.resolve(false) };
}
