/**
 * Single source of truth for status labels, badge variants, tooltips, and
 * remediation links. Backs the shared `<StatusBadge>` component and any
 * code that needs to render the same status semantics outside a badge
 * (toasts, dialog copy, etc.). Wave B item B.4.
 *
 * Why a registry, not per-domain switches:
 * - Tooltips were previously absent across every status badge in the app,
 *   so users could not learn what "Draining" / "Pending Webhook" /
 *   "Refresh Failed" meant without reading the code.
 * - Status copy drift was rampant: provider "Refresh Failed" vs
 *   "refresh_failed" code values vs the user-facing "Reconnect" verb in
 *   docs. The registry forces one canonical label per (domain, key).
 * - Adding remediation makes "what should I do about it?" trivially
 *   discoverable for the user — the badge tooltip carries the link
 *   instead of forcing the user to hunt for the relevant action.
 *
 * Add new domains / status keys here, not in component-local maps.
 */

import type { ComponentProps } from "react";
import type { Badge } from "@/components/ui/badge";

/** Variants supported by `<Badge>`. */
export type StatusVariant = NonNullable<ComponentProps<typeof Badge>["variant"]>;

/**
 * Logical domain a status key belongs to. Adding a new domain implies
 * a new section in `STATUS_REGISTRY` below and a new caller of
 * `<StatusBadge>` somewhere in the UI.
 */
export type StatusDomain =
  | "node"
  | "channel_bot"
  | "provider"
  | "user_service_credential";

/**
 * One status row. `label` is what the badge shows. `tooltip` is the
 * one-line meaning users see on hover. `remediation`, when present,
 * is a tiny link rendered in the tooltip that points the user at the
 * place they can fix the underlying problem (e.g. "Reconnect provider"
 * → `/providers`).
 */
export interface StatusMeta {
  readonly label: string;
  readonly variant: StatusVariant;
  readonly tooltip: string;
  readonly remediation?: {
    readonly label: string;
    readonly href: string;
  };
}

type StatusRegistry = {
  readonly [D in StatusDomain]: Readonly<Record<string, StatusMeta>>;
};

export const STATUS_REGISTRY: StatusRegistry = {
  node: {
    // Derived from `(isConnected, status)` — see `node-status-badge.tsx`.
    Online: {
      label: "Online",
      variant: "success",
      tooltip: "Node is connected and ready to serve proxied requests.",
    },
    Draining: {
      label: "Draining",
      variant: "warning",
      tooltip:
        "Node is finishing in-flight requests; new requests will not be routed here.",
      remediation: { label: "Manage nodes", href: "/nodes" },
    },
    Offline: {
      label: "Offline",
      variant: "secondary",
      tooltip: "Node is not currently connected to NyxID.",
      remediation: { label: "Reconnect node", href: "/nodes" },
    },
  },
  channel_bot: {
    active: {
      label: "Active",
      variant: "success",
      tooltip: "Bot is verified and receiving messages.",
    },
    pending: {
      label: "Pending",
      variant: "warning",
      tooltip: "Bot is registered; verification is in progress.",
    },
    pending_webhook: {
      label: "Pending Webhook",
      variant: "warning",
      tooltip:
        "Bot is registered but the upstream platform has not yet delivered a verified inbound. Finish the setup checklist on the bot detail page.",
    },
    failed: {
      label: "Failed",
      variant: "destructive",
      tooltip:
        "Bot verification failed on its most recent attempt. Check the bot detail page for the cause.",
    },
    invalid: {
      label: "Invalid",
      variant: "secondary",
      tooltip:
        "Bot configuration is invalid (missing or rejected credentials). Edit the bot to fix.",
    },
  },
  provider: {
    // Maps to `UserProviderToken.status` strings.
    active: {
      label: "Connected",
      variant: "success",
      tooltip: "Provider has a working token.",
    },
    expired: {
      label: "Expired",
      variant: "warning",
      tooltip:
        "Provider token expired. Reconnect to issue a fresh one.",
      remediation: { label: "Reconnect provider", href: "/providers" },
    },
    revoked: {
      label: "Revoked",
      variant: "destructive",
      tooltip:
        "Provider token was revoked. Reconnect to restore access.",
      remediation: { label: "Reconnect provider", href: "/providers" },
    },
    refresh_failed: {
      label: "Refresh Failed",
      variant: "destructive",
      tooltip:
        "NyxID could not refresh this token automatically. Reconnect to restore access.",
      remediation: { label: "Reconnect provider", href: "/providers" },
    },
  },
  user_service_credential: {
    // Outputs of `deriveServiceBadge` in lib/service-status.ts. Migrating
    // that helper to read this registry is a follow-up; for now the keys
    // here mirror the labels it already produces so callers can phase
    // their migration.
    active: {
      label: "Active",
      variant: "success",
      tooltip: "Service is enabled and its credential is healthy.",
    },
    inactive: {
      label: "Inactive",
      variant: "secondary",
      tooltip:
        "Service is disabled. Enable it from the service detail page to start routing requests.",
    },
    unavailable: {
      label: "Unavailable",
      variant: "secondary",
      tooltip:
        "Service is enabled but its credential is not in an active state. Update or reconnect the credential to resume traffic.",
    },
  },
};

/**
 * Look up the metadata for a status key. Returns `undefined` for unknown
 * keys so callers can fall through to a neutral default — e.g. a bare
 * `<Badge variant="outline">` showing the raw key — rather than throwing.
 */
export function getStatusMeta(
  domain: StatusDomain,
  key: string,
): StatusMeta | undefined {
  return STATUS_REGISTRY[domain][key];
}
