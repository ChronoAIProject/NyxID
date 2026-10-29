import { useEffect, useRef, useState } from "react";
import * as WebBrowser from "expo-web-browser";
import { connectLinksApi } from "../../lib/api/connectLinksApi";
import { servicesApi } from "../../lib/api/servicesApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type {
  CompleteConnectLinkInput,
  CompleteConnectLinkResponse,
  ConnectLinkPreview,
} from "./connectLinkContracts";
import {
  connectDeviceChallenge,
  connectPollDelay,
  type ConnectDeviceChallenge,
} from "./connectLinkState";

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : "The connection request failed.";

/** Token/credentials stay in this mounted chat dialog, never routes or caches. */
export function useHostedConnect(
  token: string,
  enabled: boolean,
  onComplete?: () => void,
) {
  const availability = useGroupAvailability();
  const [preview, setPreview] = useState<ConnectLinkPreview>();
  const [platformAvailable, setPlatformAvailable] = useState(false);
  const [iconUrl, setIconUrl] = useState<string | null>();
  const [platformChoice, setPlatformChoice] = useState<boolean | null>(null);
  const [device, setDevice] = useState<ConnectDeviceChallenge>();
  const [oauth, setOAuth] = useState<{
    id: string;
    url: string;
    platform: boolean;
  }>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);
  const owner = useRef({ token, epoch: getAuthEpoch() });
  const lastClick = useRef(-Infinity);
  const lock = useRef(false);
  const completed = useRef(false);
  const linkId = useRef<string | undefined>(undefined);
  const requests = useRef(new Set<AbortController>());
  const latest = useRef({
    token,
    enabled,
    availability,
    preview,
    device,
    onComplete,
  });
  latest.current = {
    token,
    enabled,
    availability,
    preview,
    device,
    onComplete,
  };
  const alive = () =>
    mounted.current &&
    owner.current.token === latest.current.token &&
    owner.current.epoch === getAuthEpoch();
  const current = () =>
    alive() &&
    latest.current.enabled &&
    latest.current.availability.available();
  const usePlatformKey =
    platformAvailable && (platformChoice ?? preview?.use_platform_key ?? true);
  const canAct = () =>
    current() &&
    latest.current.preview?.status === "pending" &&
    Date.parse(latest.current.preview.expires_at) > Date.now();
  async function request<T>(operation: (signal: AbortSignal) => Promise<T>) {
    const controller = new AbortController();
    requests.current.add(controller);
    try {
      return await operation(controller.signal);
    } finally {
      requests.current.delete(controller);
    }
  }
  function terminal(status: "pending" | "completed" | "cancelled" | "expired") {
    if (!current() || status === "pending") return;
    setPreview((value) => (value ? { ...value, status } : value));
    setDevice(undefined);
    setOAuth(undefined);
    if (status === "completed" && !completed.current) {
      completed.current = true;
      latest.current.onComplete?.();
    }
  }
  async function reload() {
    if (!current() || lock.current) return;
    setLoading(true);
    setIconUrl(undefined);
    try {
      const value = await request((signal) =>
        connectLinksApi.preview(token, signal),
      );
      if (!current()) return;
      setPreview((previous) =>
        previous && previous.status !== "pending" ? previous : value,
      );
      setError(undefined);
      if (value.status !== "pending") {
        terminal(value.status);
        return;
      }
      try {
        const entry = await request((signal) =>
          servicesApi.catalogEntry(value.service_slug, signal),
        );
        if (current()) {
          setIconUrl(entry.icon_url);
          setPlatformAvailable(
            !!entry.platform_key?.available && value.scopes.length === 0,
          );
        }
      } catch {
        /* A missing catalog choice never widens credential authority. */
      }
    } catch (cause) {
      if (current()) setError(errorText(cause));
    } finally {
      if (alive()) setLoading(false);
    }
  }
  function applyResult(result: CompleteConnectLinkResponse, platform: boolean) {
    if (!current()) return;
    if (linkId.current && result.id !== linkId.current)
      throw new Error(
        "The server returned a different connection request. Please reload.",
      );
    linkId.current = result.id;
    // Completed links can return a provisioned UserService slug, not the catalog slug.
    if (
      result.status !== "completed" &&
      result.service_slug !== latest.current.preview?.service_slug
    )
      throw new Error(
        "The server returned a different service. Please reload.",
      );
    if (result.status === "completed") {
      terminal("completed");
      return;
    }
    if (result.status === "device_code_required") {
      setDevice(connectDeviceChallenge(result, latest.current.device));
      setOAuth(undefined);
    } else {
      const url = providerUrl(result.authorization_url);
      if (!url)
        throw new Error(
          "The provider did not return a safe authorization URL. Try again.",
        );
      setOAuth({ id: result.id, url, platform });
      setDevice(undefined);
    }
  }
  async function checkOAuth(value: NonNullable<typeof oauth>, recover = false) {
    if (!current()) return;
    const status = await request((signal) =>
      connectLinksApi.status(value.id, signal),
    );
    if (!current()) return;
    if (
      status.status !== "completed" &&
      status.service_slug !== latest.current.preview?.service_slug
    )
      throw new Error(
        "The server returned a different service. Please reload.",
      );
    terminal(status.status);
    if (status.status === "pending" && recover && canAct()) {
      // Same recovery call as the web return page. Browser dismissal is never
      // completion evidence: only the server's terminal response settles it.
      const result = await request((signal) =>
        connectLinksApi.complete(
          token,
          { use_platform_key: value.platform },
          signal,
        ),
      );
      applyResult(result, value.platform);
    }
    if (current() && status.last_error) setError(status.last_error);
  }
  async function openProvider(url: string) {
    const safe = providerUrl(url);
    if (!safe || !current()) return;
    try {
      await WebBrowser.openBrowserAsync(safe);
    } catch {
      if (current()) setError("Could not open the provider. Try again.");
    }
  }
  async function act(
    kind: "approve" | "decline" | "check",
    values: CompleteConnectLinkInput = {},
  ) {
    if (!canAct() || lock.current || Date.now() - lastClick.current < 750)
      return;
    lastClick.current = Date.now();
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      if (kind === "decline") {
        const result = await request((signal) =>
          connectLinksApi.cancel(token, signal),
        );
        if (
          current() &&
          result.service_slug !== latest.current.preview?.service_slug
        )
          throw new Error(
            "The server returned a different service. Please reload.",
          );
        terminal(result.status);
      } else if (kind === "check" && oauth) {
        await checkOAuth(oauth, true);
      } else {
        const input = usePlatformKey
          ? { use_platform_key: true }
          : { ...values, use_platform_key: false };
        const result = await request((signal) =>
          connectLinksApi.complete(token, input, signal),
        );
        applyResult(result, usePlatformKey);
        if (current() && result.status === "oauth_required") {
          const url = providerUrl(result.authorization_url);
          if (url) {
            await openProvider(url);
            if (current())
              await checkOAuth(
                { id: result.id, url, platform: usePlatformKey },
                true,
              );
          }
        }
      }
    } catch (cause) {
      if (current()) {
        setError(errorText(cause));
        try {
          const fresh = await request((signal) =>
            connectLinksApi.preview(token, signal),
          );
          if (current() && fresh.status !== "pending") terminal(fresh.status);
        } catch {
          /* Preserve the original action error when preview also fails. */
        }
      }
    } finally {
      lock.current = false;
      if (alive()) setBusy(false);
    }
  }
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      for (const controller of requests.current) controller.abort();
      requests.current.clear();
    };
  }, []);
  useEffect(() => {
    if (enabled && availability.active) void reload();
    return () => {
      for (const controller of requests.current) controller.abort();
    };
  }, [token, enabled, availability.active]);
  useEffect(() => {
    if (!enabled || !availability.active || preview?.status !== "pending")
      return;
    const remaining = Date.parse(preview.expires_at) - Date.now();
    if (!Number.isFinite(remaining)) return;
    const timer = setTimeout(
      () => {
        if (current() && Date.parse(preview.expires_at) <= Date.now())
          terminal("expired");
      },
      Math.max(0, Math.min(remaining, 2_147_483_647)),
    );
    return () => clearTimeout(timer);
  }, [enabled, availability.active, preview?.status, preview?.expires_at]);
  useEffect(() => {
    if (
      !enabled ||
      !availability.active ||
      preview?.status !== "pending" ||
      (!device && !oauth)
    )
      return;
    let stopped = false;
    let failures = 0;
    let timer: ReturnType<typeof setTimeout>;
    function schedule(seconds: number) {
      timer = setTimeout(() => void poll(), seconds * 1000);
    }
    async function poll() {
      if (stopped || !canAct()) return;
      let interval = latest.current.device?.interval ?? 1;
      if (lock.current) {
        schedule(interval);
        return;
      }
      lock.current = true;
      try {
        if (device) {
          const result = await request((signal) =>
            connectLinksApi.complete(
              token,
              { device_state: device.state, use_platform_key: usePlatformKey },
              signal,
            ),
          );
          if (stopped || !current()) return;
          applyResult(result, usePlatformKey);
          setError(undefined);
          interval = result.device_interval ?? interval;
        } else if (oauth) {
          await checkOAuth(oauth);
        }
        failures = 0;
      } catch (cause) {
        if (stopped || !current()) return;
        const next = connectPollDelay(cause, ++failures, interval);
        if (next === null) {
          setError(errorText(cause));
          return;
        }
        interval = next;
      } finally {
        lock.current = false;
      }
      if (!stopped && canAct()) schedule(Math.max(1, interval));
    }
    schedule(device?.interval ?? 1);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [
    token,
    enabled,
    availability.active,
    preview?.status,
    device?.state,
    oauth?.id,
  ]);
  return {
    preview,
    loading,
    busy,
    error,
    setError,
    platformAvailable,
    iconUrl,
    usePlatformKey,
    setPlatformChoice,
    device,
    oauth,
    reload,
    approve: (values: CompleteConnectLinkInput) => act("approve", values),
    decline: () => act("decline"),
    check: () => act("check"),
    openProvider,
    disabled:
      !enabled ||
      !availability.active ||
      busy ||
      preview?.status !== "pending" ||
      Date.parse(preview.expires_at) <= Date.now(),
  };
}
export function providerUrl(raw?: string | null): string | undefined {
  try {
    const url = new URL(raw ?? "");
    if (url.protocol === "https:" && !url.username && !url.password)
      return url.href;
  } catch {
    /* Invalid provider URL stays inert. */
  }
  return undefined;
}
