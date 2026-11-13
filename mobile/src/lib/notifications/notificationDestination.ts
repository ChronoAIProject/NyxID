export type NotificationLinkConfig = {
  scheme: string;
  trustedUrls: readonly string[];
};
const string = (value: unknown) =>
  typeof value === "string" && value.trim() ? value.trim() : undefined;

/** Normalize server browser destinations before passing them to the native router. */
export function nativeNotificationUrl(
  value: string | null | undefined,
  config: NotificationLinkConfig,
): string | null {
  if (!value) return null;
  let path: string;
  try {
    const url =
      value.startsWith("/") && config.trustedUrls[0]
        ? new URL(value, config.trustedUrls[0])
        : new URL(value);
    if (url.username || url.password || url.hash) return null;
    if (url.protocol === `${config.scheme}:`) {
      if (!url.host && (!url.pathname || url.pathname === "/")) return null;
      path = `${url.host ? `/${url.host}` : ""}${url.pathname && !url.pathname.startsWith("/") ? "/" : ""}${url.pathname}${url.search}`;
    } else {
      if (
        !config.trustedUrls.some((base) => new URL(base).origin === url.origin)
      )
        return null;
      path = `${url.pathname}${url.search}`;
    }
    const parsed = new URL(path, "https://native.invalid");
    decodeURIComponent(parsed.pathname);
    const challenge = /^\/challenge\/([^/]+)$/.exec(parsed.pathname);
    if (challenge)
      path = `/activity?${new URLSearchParams({ challengeId: decodeURIComponent(challenge[1]!) })}`;
    else if (/^\/machines\/?$/.test(parsed.pathname))
      path = `/assistant/machines${parsed.search}`;
    else if (/^\/channel-bots(?:\/[^/]+)?\/?$/.test(parsed.pathname))
      path = "/plugins?tab=connectors";
    else if (/^\/keys\/?$/.test(parsed.pathname))
      path = "/plugins?tab=connectors";
    return `${config.scheme}:/${path}`;
  } catch {
    return null;
  }
}

/** APNs and FCM can wrap data as objects or JSON strings; bound both traversal and input. */
export function notificationDestination(
  data: unknown,
  config: NotificationLinkConfig,
): string | null {
  const pending: unknown[] = [data];
  const visited = new Set<object>();
  for (let count = 0; pending.length && count < 64; count++) {
    let current = pending.shift();
    if (typeof current === "string" && current.length <= 65536) {
      try {
        current = JSON.parse(current);
      } catch {
        continue;
      }
    }
    if (!current || typeof current !== "object" || visited.has(current))
      continue;
    visited.add(current);
    const row = current as Record<string, unknown>;
    const conversation = string(row.conversation_id) ?? string(row.thread_id);
    const group =
      string(row.group_id) ??
      (conversation?.startsWith("nyxg-") ? conversation : undefined);
    const agent = string(row.agent_id);
    const card = string(row.acknowledgement_id) ?? string(row.card_id);
    let path: string | undefined;
    if (group) path = `groups/${encodeURIComponent(group)}`;
    else if (conversation) {
      const query = new URLSearchParams({
        conversationId: conversation,
        draft: "false",
        openLatest: "false",
      });
      if (agent) query.set("agentId", agent);
      if (card) query.set("acknowledgementId", card);
      path = `chat?${query}`;
    } else {
      const challenge =
        string(row.challenge_id) ??
        string(row.challengeId) ??
        (row.type === "approval_request" ||
        row.type === "approval_decision" ||
        row.type === "approval_expired" ||
        !row.type
          ? string(row.request_id)
          : undefined);
      if (challenge)
        path = `activity?${new URLSearchParams({ challengeId: challenge })}`;
    }
    if (path) return `${config.scheme}://${path}`;
    for (const key of [
      "deeplink",
      "deep_link",
      "url",
      "link",
      "review_path",
      "reconnect_url",
    ]) {
      const href = string(row[key]);
      if (!href) continue;
      const normalized = nativeNotificationUrl(href, config);
      if (normalized) return normalized;
    }
    if (agent)
      return `${config.scheme}://chat?${new URLSearchParams({ agentId: agent, openLatest: "true" })}`;
    if (row.type === "channel_bot_failed")
      return `${config.scheme}://plugins?tab=connectors`;
    if (row.type === "machine_lifecycle" || row.type === "machine_control")
      return `${config.scheme}://assistant/machines${string(row.machine_id) ? `?${new URLSearchParams({ machine: string(row.machine_id)! })}` : ""}`;
    // Credential-node device notifications have no native device detail route.
    if (
      [
        "device_bind_success",
        "device_repeated_fail",
        "device_lock_alert",
      ].includes(String(row.type))
    )
      return `${config.scheme}://activity`;
    if (row.type === "connection_expired")
      return `${config.scheme}://plugins?tab=connectors`;
    if (row.type === "mfa_disabled" || row.type === "mfa_recovery_used")
      return `${config.scheme}://account/security`;
    if (row.type === "trigger_event") return `${config.scheme}://automations`;
    if (
      row.type === "approval_request" ||
      row.type === "approval_decision" ||
      row.type === "approval_expired"
    )
      return `${config.scheme}://activity`;
    for (const child of Object.values(row))
      if (
        typeof child === "object" ||
        (typeof child === "string" && /^[{[]/.test(child))
      )
        pending.push(child);
  }
  return null;
}
