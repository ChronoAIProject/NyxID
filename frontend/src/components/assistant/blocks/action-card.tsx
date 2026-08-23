import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import {
  AlertTriangle,
  AppWindow,
  Bell,
  Building2,
  Globe,
  KeyRound,
  Loader2,
  Server,
  ShieldCheck,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  dialogBindingFor,
  isDialogParams,
} from "@/components/assistant/blocks/action-dialogs";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useChatPresence } from "@/hooks/use-chat-presence";
import { KEY_AUTH_FAILED, useKeyAuthorizationWatch } from "@/hooks/use-keys";
import {
  descriptorForAction,
  type ActionIcon,
  type SummaryRow,
} from "@/lib/assistant/action-registry";
import { connectWatchDeadline } from "@/lib/assistant/connect-watch";
import { ApiError, api } from "@/lib/api-client";
import type { ActionReport, ActionResource } from "@/schemas/assistant-actions";
import { usePendingConnectStore } from "@/stores/pending-connect-store";
import type { ActionCardContentBlock } from "@/types/assistant";
import type { KeyInfo } from "@/types/keys";

interface ActionCardProps {
  readonly block: ActionCardContentBlock;
  readonly onProgress: (blockId: string, inProgress: boolean) => void;
  readonly onBlock: (blockId: string, note: string) => Promise<void> | void;
  readonly onResolve: (report: ActionReport) => Promise<void> | void;
}

const VERIFICATION_BLOCKED_NOTE =
  "Connected, but NyxID could not verify which service was created. Manage it in AI Services, then ask the assistant to request it again.";

const AUTHORIZATION_TIMEOUT_NOTE =
  "NyxID stopped waiting for this connection to finish authorizing. If you did complete it, find it in AI Services — otherwise ask the assistant to request it again.";

const REAUTHORIZE_NOT_FOUND_NOTE =
  "NyxID could not find that connected service. Ask the assistant to refresh its service list and request re-authorization again.";
const REAUTHORIZE_UNAVAILABLE_NOTE =
  "NyxID could not verify this service for re-authorization. Manage it in AI Services, then ask the assistant to request it again.";
const REAUTHORIZE_IDENTITY_NOTE =
  "NyxID returned a different connected service than the one requested. The re-authorization was blocked.";
const REAUTHORIZE_INACTIVE_NOTE =
  "This service is disabled. Enable it in AI Services before re-authorizing it.";
const REAUTHORIZE_CREDENTIAL_NOTE =
  "This service does not have a usable OAuth credential to re-authorize. Repair it in AI Services first.";
const REAUTHORIZE_MODALITY_NOTE =
  "This service does not support browser re-authorization. Manage its credential in AI Services instead.";
const REAUTHORIZE_PLATFORM_NOTE =
  "Platform-managed services cannot be re-authorized from an assistant request.";
const REAUTHORIZE_ORG_NOTE =
  "Only an organization admin can re-authorize this shared service.";
const REAUTHORIZE_CATALOG_NOTE =
  "NyxID could not resolve this service's OAuth provider. Manage it in AI Services, then ask the assistant to try again.";
const REAUTHORIZE_SCOPES_UNSUPPORTED_NOTE =
  "This provider does not accept requested scope changes during device authorization, so NyxID did not start a re-authorization that could drop the request.";
const REAUTHORIZE_EVIDENCE_UNREADABLE_NOTE =
  "NyxID could not read this service's authorization state, so the re-authorization was not confirmed. Check it in AI Services, then ask the assistant to request it again.";
const REAUTHORIZE_SECRET_EVIDENCE_NOTE =
  "NyxID returned credential-shaped data where it should have returned only authorization state, so the re-authorization was not confirmed. This is a NyxID bug — report it rather than retrying.";

/** Ordinal, matching the postcondition reader's `StringComparer.Ordinal`. */
function missingScopes(
  requested: readonly string[],
  granted: readonly string[] | null,
): readonly string[] {
  if (granted === null) return [...requested];
  const held = new Set(granted);
  return requested.filter((scope) => !held.has(scope));
}

function scopeShortfallNote(missing: readonly string[]): string {
  return `The provider did not grant ${missing.join(", ")}. NyxID did not report this as re-authorized — ask the assistant to request it again, or grant the missing access at the provider first.`;
}

const reauthorizationCredentialSourceSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("personal") }).passthrough(),
  z
    .object({
      type: z.literal("org"),
      role: z.string(),
    })
    .passthrough(),
]);
const reauthorizationKeySchema = z
  .object({
    id: z.string().min(1).max(256),
    slug: z.string().min(1),
    api_key_id: z.string().min(1).optional(),
    credential_missing: z.boolean().optional(),
    credential_type: z.string(),
    auth_method: z.string(),
    is_active: z.boolean(),
    auto_connected: z.boolean(),
    catalog_service_slug: z.string().nullish(),
    // Required, not optional. `KeyResponse.credential_source` is mandatory on
    // the wire, and an `.optional()` schema would let the org-admin guard
    // below silently vanish if that ever changed — the opposite of what a
    // defence-in-depth check is for. The backend remains the real gate.
    credential_source: reauthorizationCredentialSourceSchema,
  })
  .passthrough();
const reauthorizationCatalogEntrySchema = z
  .object({
    slug: z.string().min(1),
    provider_type: z.string().nullish(),
    provider_config_id: z.string().nullish(),
    device_code_format: z.string().nullish(),
  })
  .passthrough();

/**
 * The authorization-evidence projection of a user service
 * (`GET /keys/{id}/authorization`) — the same seven properties the
 * assistant-side postcondition reader consumes.
 */
const authorizationEvidenceSchema = z
  .object({
    id: z.string().min(1),
    api_key_id: z.string().min(1).nullish(),
    is_active: z.boolean(),
    status: z.string().min(1),
    connection_status: z.string().nullable(),
    granted_scopes: z.array(z.string()).nullable(),
    last_authorized_at: z.string().nullable(),
  })
  .strict();

type AuthorizationEvidence = z.infer<typeof authorizationEvidenceSchema>;

const FORBIDDEN_READ_BACK_FIELDS = new Set([
  "apikey",
  "fullkey",
  "keyhash",
  "credential",
  "credentials",
  "accesstoken",
  "refreshtoken",
  "authorization",
  "cookie",
  "cookies",
  "secret",
  "secrets",
  "clientsecret",
  "password",
  "token",
  "passphrase",
  "usercode",
  "devicecode",
  "rawbody",
  "rawupstreambody",
]);
const SECRET_READ_BACK_VALUE =
  /(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})/i;

function assertSecretFreeReadBack(value: unknown): void {
  if (typeof value === "string" && SECRET_READ_BACK_VALUE.test(value)) {
    throw new Error("NyxID returned secret-bearing verification data.");
  }
  if (Array.isArray(value)) {
    for (const entry of value) assertSecretFreeReadBack(entry);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, entry] of Object.entries(value)) {
    const normalized = key
      .replace(/[^A-Za-z0-9]/g, "")
      .toLocaleLowerCase("en-US");
    if (FORBIDDEN_READ_BACK_FIELDS.has(normalized)) {
      throw new Error("NyxID returned secret-bearing verification data.");
    }
    assertSecretFreeReadBack(entry);
  }
}

class ReauthorizationBlockedError extends Error {}

/** A secret-shaped value reached a read that reports to the assistant. */
class SecretBearingEvidenceError extends Error {}

/**
 * Read the authorization evidence for one user service.
 *
 * This is the read that decides whether the assistant is told the
 * re-authorization happened, so it is the read the secret-shape assertion
 * belongs on — and it is served as a minimal projection precisely so a
 * legitimately configured service (a `Bearer ${credential}` WS auth template,
 * a `Bearer …` service label) cannot make its own authorization permanently
 * unverifiable. The equivalent backend assertion over a fully populated
 * detail response is the canary for the projection itself.
 */
async function readAuthorizationEvidence(
  userServiceId: string,
): Promise<AuthorizationEvidence> {
  const value = await api.get<unknown>(
    `/keys/${encodeURIComponent(userServiceId)}/authorization`,
  );
  try {
    assertSecretFreeReadBack(value);
  } catch {
    throw new SecretBearingEvidenceError(REAUTHORIZE_SECRET_EVIDENCE_NOTE);
  }
  const evidence = authorizationEvidenceSchema.parse(value);
  if (evidence.id !== userServiceId) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_IDENTITY_NOTE);
  }
  return evidence;
}

/**
 * Verify that the provider actually granted every requested scope before the
 * card reports `completed`.
 *
 * `last_authorized_at` advancing proves an authorization landed, not that it
 * granted anything new: the token endpoint may omit `scope`, in which case the
 * backend deliberately preserves the previous `token_scopes` while still
 * stamping the timestamp. Returns the note to block with, or `null` to
 * proceed.
 */
async function scopeGrantShortfall(
  userServiceId: string,
  requestedScopes: readonly string[],
): Promise<string | null> {
  if (requestedScopes.length === 0) return null;
  let evidence: AuthorizationEvidence;
  try {
    evidence = await readAuthorizationEvidence(userServiceId);
  } catch (caught) {
    if (
      caught instanceof SecretBearingEvidenceError ||
      caught instanceof ReauthorizationBlockedError
    ) {
      return caught.message;
    }
    return REAUTHORIZE_EVIDENCE_UNREADABLE_NOTE;
  }
  const missing = missingScopes(requestedScopes, evidence.granted_scopes);
  return missing.length > 0 ? scopeShortfallNote(missing) : null;
}

async function readReauthorizationKey(userServiceId: string): Promise<KeyInfo> {
  const value = await api.get<unknown>(
    `/keys/${encodeURIComponent(userServiceId)}`,
  );
  // Deliberately not secret-scanned. This is the eligibility read: nothing
  // from it is reported to the assistant, and the full detail response
  // legitimately carries user free text (service label, custom header values,
  // the supported `Bearer ${credential}` WS template) that a secret-shape
  // scan cannot tell from a real credential. Scanning it here would refuse to
  // start the journey for services that are configured perfectly correctly.
  // The assertion lives on `readAuthorizationEvidence` instead.
  const snapshot = reauthorizationKeySchema.parse(value);
  if (snapshot.id !== userServiceId) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_IDENTITY_NOTE);
  }
  if (!snapshot.is_active) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_INACTIVE_NOTE);
  }
  if (snapshot.credential_missing || !snapshot.api_key_id) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_CREDENTIAL_NOTE);
  }
  if (
    snapshot.credential_type !== "oauth2" &&
    snapshot.auth_method !== "oauth2" &&
    snapshot.auth_method !== "oidc"
  ) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_MODALITY_NOTE);
  }
  if (snapshot.auto_connected) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_PLATFORM_NOTE);
  }
  if (
    snapshot.credential_source.type === "org" &&
    snapshot.credential_source.role !== "admin"
  ) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_ORG_NOTE);
  }

  // One entry, not the whole catalog: `/catalog/{slug}` returns the same
  // `provider_type` / `provider_config_id` / `device_code_format` this needs,
  // and also resolves rows the list endpoint filters out.
  const catalogSlug = snapshot.catalog_service_slug ?? snapshot.slug;
  let catalogEntry: z.infer<typeof reauthorizationCatalogEntrySchema>;
  try {
    catalogEntry = reauthorizationCatalogEntrySchema.parse(
      await api.get<unknown>(`/catalog/${encodeURIComponent(catalogSlug)}`),
    );
  } catch (caught) {
    if (caught instanceof ApiError && caught.status === 404) {
      throw new ReauthorizationBlockedError(REAUTHORIZE_CATALOG_NOTE);
    }
    throw caught;
  }
  if (
    (catalogEntry.provider_type !== "oauth2" &&
      catalogEntry.provider_type !== "device_code") ||
    !catalogEntry.provider_config_id
  ) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_MODALITY_NOTE);
  }
  if (
    catalogEntry.provider_type === "device_code" &&
    catalogEntry.device_code_format === "openai"
  ) {
    throw new ReauthorizationBlockedError(REAUTHORIZE_SCOPES_UNSUPPORTED_NOTE);
  }

  // The public KeyInfo type is the full response contract. This read validates
  // the journey-owned projection above and preserves the remaining fields for
  // the existing reconnect dialog, which already consumes that full contract.
  return value as KeyInfo;
}

function authorizationFailedNote(reason: string | undefined): string {
  const detail = reason?.trim();
  return detail
    ? `Authorization did not complete: ${detail}`
    : "Authorization did not complete. Ask the assistant to request this service again.";
}

function groupSummaryRows(
  rows: readonly SummaryRow[],
): readonly SummaryRow[][] {
  const groups: SummaryRow[][] = [];
  for (const row of rows) {
    const current = groups.at(-1);
    if (current?.[0]?.label === row.label) current.push(row);
    else groups.push([row]);
  }
  return groups;
}

function ParameterSummary({ rows }: { readonly rows: readonly SummaryRow[] }) {
  if (rows.length === 0) return null;
  return (
    <div className="space-y-2.5 border-y border-border bg-muted px-4 py-3">
      {groupSummaryRows(rows).map((group, groupIndex) => (
        <div
          key={`${group[0]?.label ?? "summary"}-${groupIndex}`}
          className="flex flex-wrap items-center gap-1.5"
        >
          {group[0]?.label ? (
            <span className="text-[10px] font-semibold uppercase tracking-[1px] text-muted-foreground">
              {group[0].label}
            </span>
          ) : null}
          {group.map((row, rowIndex) => (
            <Badge
              key={`${row.value}-${rowIndex}`}
              variant="secondary"
              className={`max-w-full truncate${row.mono ? " font-mono" : ""}`}
            >
              <span className="min-w-0 truncate">{row.value}</span>
            </Badge>
          ))}
        </div>
      ))}
    </div>
  );
}

const ACTION_ICON_COMPONENTS: Readonly<
  Record<Exclude<ActionIcon, "service">, LucideIcon>
> = {
  globe: Globe,
  shield: ShieldCheck,
  key: KeyRound,
  org: Building2,
  bell: Bell,
  app: AppWindow,
  node: Server,
};

function DescriptorIcon({
  icon,
  params,
}: {
  readonly icon: ActionIcon;
  readonly params: ActionCardContentBlock["params"];
}) {
  if (icon === "service" && params.variant === "catalog") {
    return <ServiceIcon slug={params.service_slug} size="sm" />;
  }
  const Icon = icon === "service" ? Globe : ACTION_ICON_COMPONENTS[icon];
  return <Icon className="h-4 w-4 text-muted-foreground" />;
}

/**
 * A settled card keeps the whole connect card — service icon, title, scopes,
 * routing — and swaps only its verdict surfaces. Collapsing it to a bare
 * receipt used to erase what the user had just agreed to.
 */
const SETTLED = {
  completed: {
    badge: "Connected",
    badgeVariant: "success",
    frame: "border-success/30 bg-success/10",
    icon: ShieldCheck,
    iconClass: "text-success",
    footer:
      "Your credential stays in NyxID. The assistant only received a reference to this service.",
  },
  declined: {
    badge: "Declined",
    badgeVariant: "secondary",
    frame: "border-border bg-overlay",
    icon: X,
    iconClass: "text-muted-foreground",
    footer: null,
  },
  failed: {
    badge: "Failed",
    badgeVariant: "destructive",
    frame: "border-destructive/30 bg-destructive/10",
    icon: AlertTriangle,
    iconClass: "text-destructive",
    footer: null,
  },
} as const;

type SettledStatus = keyof typeof SETTLED;

function StatusNotice({ block }: { readonly block: ActionCardContentBlock }) {
  if (block.status !== "blocked" && block.status !== "conflicted") {
    return null;
  }
  return (
    <div className="flex items-start gap-2 border-t border-border bg-muted px-4 py-3">
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        {block.outcome_note}
      </p>
    </div>
  );
}

export function ActionCard({
  block,
  onProgress,
  onBlock,
  onResolve,
}: ActionCardProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [reauthorizeKey, setReauthorizeKey] = useState<KeyInfo | null>(null);
  const resolvingRef = useRef(false);
  const journeyStartingRef = useRef(false);
  /**
   * An out-of-band authorization handed to a provider, still settling. Held
   * outside React (keyed by block id) rather than in the dialog or in this
   * component: the dialog is the short-lived surface — the user goes to
   * GitHub, comes back to the chat, and never reopens it — and the card
   * itself outlives neither a conversation switch nor a history refetch that
   * re-keys message groups. The busy projection it clears lives in the
   * transport mirror and survives both, so the watch has to as well.
   */
  const pendingAuth = usePendingConnectStore(
    (state) => state.attempts[block.block_id] ?? null,
  );
  const beginPendingAuth = usePendingConnectStore((state) => state.begin);
  const endPendingAuth = usePendingConnectStore((state) => state.end);
  /** Guards one auto-settlement per authorization attempt against effect re-entry. */
  const watchSettledRef = useRef<string | null>(null);
  /** One mount-time reconciliation, whatever the deps churn afterwards. */
  const reconciledRef = useRef(false);
  const { visible, lastActivityAt } = useChatPresence();

  useEffect(() => {
    if (block.status === "pending") {
      resolvingRef.current = false;
    }
  }, [block.status]);

  // A card that mounts busy with no authorization behind it and no dialog open
  // was stranded: `in_progress` disables every control, and its only writers
  // (dialog dismissal, the watch, an explicit report) are all gone. Roll it
  // back to actionable instead of leaving a "Connecting" spinner nobody can
  // clear. Fresh cards mount `pending`, and a remount mid-authorization finds
  // its attempt in the store, so neither is touched.
  useEffect(() => {
    if (reconciledRef.current) return;
    reconciledRef.current = true;
    if (block.status !== "in_progress" || pendingAuth !== null) return;
    onProgress(block.block_id, false);
  }, [block.block_id, block.status, pendingAuth, onProgress]);

  const settled =
    block.status === "completed" ||
    block.status === "declined" ||
    block.status === "failed";

  // Typed, non-persisted records always carry an attempt id. Keep malformed
  // runtime data settleable anyway, with a fallback isolated to this card.
  const authorizationAttemptId =
    pendingAuth?.attemptId ?? `missing:${block.block_id}`;
  const watch = useKeyAuthorizationWatch(pendingAuth?.keyId ?? null, {
    attemptId: authorizationAttemptId,
    previousAuthorizationAt: pendingAuth?.previousAuthorizationAt,
    // Presence gate: a hidden tab stops polling and resumes on focus.
    enabled: pendingAuth !== null && !settled && visible,
    deadlineAt: pendingAuth
      ? connectWatchDeadline(pendingAuth.startedAt, lastActivityAt)
      : 0,
  });

  // The card's own `wait_for_authorized_key`. `active` is the same verdict the
  // user would have produced by clicking through Continue and Done, so take it
  // directly instead of requiring those clicks; `failed` and the deadline both
  // surface on the card rather than leaving it waiting in silence.
  useEffect(() => {
    const keyId = pendingAuth?.keyId;
    const attemptId = pendingAuth ? authorizationAttemptId : null;
    if (
      !keyId ||
      !attemptId ||
      settled ||
      watchSettledRef.current === attemptId
    ) {
      return;
    }

    // The ref is the synchronous re-entry guard; the state clear rides the
    // microtask so this effect never sets the state it depends on inline.
    if (watch.authorized) {
      if (
        block.params.variant === "service_reauthorize" &&
        keyId !== block.params.user_service_id
      ) {
        watchSettledRef.current = attemptId;
        void Promise.resolve()
          .then(() => {
            endPendingAuth(block.block_id);
            return onBlock(block.block_id, REAUTHORIZE_IDENTITY_NOTE);
          })
          .catch(() => undefined);
        return;
      }
      watchSettledRef.current = attemptId;
      resolvingRef.current = true;
      const requestedScopes =
        block.params.variant === "service_reauthorize"
          ? block.params.requested_scopes
          : [];
      void Promise.resolve()
        .then(async () => {
          endPendingAuth(block.block_id);
          // A fresh authorization is not the same as a granted one. Confirm
          // the provider actually issued every requested scope before telling
          // the assistant this succeeded.
          const shortfall = await scopeGrantShortfall(keyId, requestedScopes);
          if (shortfall) {
            resolvingRef.current = false;
            return onBlock(block.block_id, shortfall);
          }
          return onResolve({
            actionRequestId: block.action_request_id,
            originTurnId: block.origin_turn_id,
            disposition: "completed",
            resource: { userService: { userServiceId: keyId } },
          });
        })
        .catch(() => {
          resolvingRef.current = false;
          onProgress(block.block_id, false);
        });
      return;
    }

    if (watch.status === KEY_AUTH_FAILED || watch.timedOut) {
      watchSettledRef.current = attemptId;
      const note =
        watch.status === KEY_AUTH_FAILED
          ? authorizationFailedNote(watch.errorMessage)
          : AUTHORIZATION_TIMEOUT_NOTE;
      void Promise.resolve()
        .then(() => {
          endPendingAuth(block.block_id);
          return onBlock(block.block_id, note);
        })
        .catch(() => undefined);
    }
  }, [
    pendingAuth,
    authorizationAttemptId,
    settled,
    watch.status,
    watch.authorized,
    watch.timedOut,
    watch.errorMessage,
    block.params,
    block.block_id,
    block.action_request_id,
    block.origin_turn_id,
    endPendingAuth,
    onResolve,
    onBlock,
    onProgress,
  ]);

  const descriptor = descriptorForAction(
    block.action,
    block.params,
    block.status !== "unsupported",
  );

  const baseVerdict = settled ? SETTLED[block.status as SettledStatus] : null;
  const verdict =
    baseVerdict &&
    block.status === "completed" &&
    block.action === "service.reauthorize"
      ? {
          ...baseVerdict,
          badge: "Re-authorized",
          footer:
            "The assistant received only the verified service reference. Your OAuth credential stayed in NyxID.",
        }
      : baseVerdict &&
          block.status === "completed" &&
          block.action === "key.create"
        ? {
            ...baseVerdict,
            badge: "Created",
            footer:
              "The assistant received only the verified key reference. Key material stayed in NyxID.",
          }
        : baseVerdict &&
            block.status === "completed" &&
            block.action === "key.rotate"
          ? {
              ...baseVerdict,
              badge: "Rotated",
              footer:
                "The assistant received only the verified replacement key reference. Replacement key material stayed in NyxID.",
            }
          : baseVerdict;
  const VerdictIcon = verdict?.icon;

  // Trust the descriptor too: a card whose verb has no journey behind it must
  // never render a CTA, whatever status the block carries.
  const unsupported =
    block.status === "unsupported" || descriptor.risk === "unsupported";
  const busy = block.status === "in_progress";
  /** Dialog dismissed, provider not finished: the watch is carrying this. */
  const awaitingAuthorization = pendingAuth !== null && !dialogOpen;
  const blocked = block.status === "blocked";
  const conflicted = block.status === "conflicted";
  const primaryDisabled = busy || blocked || conflicted;
  // Decline stays live through `in_progress`. Abandoning a connection the user
  // started is always their call, and it is the manual floor under every
  // automatic settlement: with it disabled, a busy card that lost its watch
  // had no reachable control at all. `report` supersedes any watch still
  // running, and the transport de-duplicates a report already queued.
  const secondaryDisabled = conflicted;
  const params = block.params;

  function setOpen(next: boolean) {
    setDialogOpen(next);
    // Closing the dialog on a still-settling authorization is NOT abandonment:
    // the provider tab is open, the watch is running, and the card resolves
    // itself. Rolling back to `pending` here is what used to lose a connection
    // the user had actually completed — and invite a duplicate on the retry.
    if (
      !next &&
      !resolvingRef.current &&
      !pendingAuth &&
      block.status === "in_progress"
    ) {
      onProgress(block.block_id, false);
    }
  }

  function report(
    disposition: "completed" | "declined" | "failed",
    resource?: ActionResource,
  ) {
    // A manual outcome supersedes any watch still running for this card.
    if (pendingAuth) {
      watchSettledRef.current = authorizationAttemptId;
      endPendingAuth(block.block_id);
    }
    if (disposition === "completed" && !resource) {
      resolvingRef.current = true;
      void Promise.resolve()
        .then(() => onBlock(block.block_id, VERIFICATION_BLOCKED_NOTE))
        .catch(() => undefined)
        .finally(() => {
          resolvingRef.current = false;
        });
      return;
    }
    resolvingRef.current = true;
    const base = {
      actionRequestId: block.action_request_id,
      originTurnId: block.origin_turn_id,
      disposition,
    } as const;
    void Promise.resolve()
      .then(() =>
        onResolve(
          disposition === "completed" && resource
            ? { ...base, resource }
            : base,
        ),
      )
      .catch(() => {
        // The transport retains failed/rejected reports for retry, and the
        // page has already toasted the delivery failure. Unlock dismissal AND
        // roll the card out of any busy projection: a completed-connection
        // report that dies must not strand the card at "Connecting" with its
        // controls disabled — back to actionable is what makes retry possible.
        resolvingRef.current = false;
        onProgress(block.block_id, false);
      });
  }

  async function beginJourney() {
    if (journeyStartingRef.current) return;
    journeyStartingRef.current = true;
    onProgress(block.block_id, true);

    if (params.variant !== "service_reauthorize") {
      setDialogOpen(true);
      journeyStartingRef.current = false;
      return;
    }

    try {
      const key = await readReauthorizationKey(params.user_service_id);
      setReauthorizeKey(key);
      setDialogOpen(true);
    } catch (caught) {
      const note =
        caught instanceof ReauthorizationBlockedError
          ? caught.message
          : caught instanceof ApiError && caught.status === 404
            ? REAUTHORIZE_NOT_FOUND_NOTE
            : REAUTHORIZE_UNAVAILABLE_NOTE;
      try {
        await onBlock(block.block_id, note);
      } catch {
        onProgress(block.block_id, false);
      }
    } finally {
      journeyStartingRef.current = false;
    }
  }

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm">
      <div className="flex items-start gap-3 px-4 py-3.5">
        <div
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border ${
            verdict
              ? verdict.frame
              : unsupported
                ? "border-destructive/30 bg-destructive/10"
                : "border-nyx-secondary-400/30 bg-nyx-secondary-400/10"
          }`}
        >
          {unsupported ? (
            <AlertTriangle className="h-4 w-4 text-destructive" />
          ) : (
            <DescriptorIcon icon={descriptor.icon} params={params} />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-[13px] font-semibold text-foreground">
              {descriptor.title(params)}
            </h3>
            <Badge
              variant={
                verdict
                  ? verdict.badgeVariant
                  : unsupported || conflicted
                    ? "destructive"
                    : blocked
                      ? "warning"
                      : "accent"
              }
            >
              {verdict
                ? verdict.badge
                : unsupported
                  ? "Unsupported"
                  : conflicted
                    ? "Conflict"
                    : blocked
                      ? "Blocked"
                      : awaitingAuthorization
                        ? "Authorizing"
                        : busy
                          ? "In progress"
                          : "Action required"}
            </Badge>
          </div>
          <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">
            {/* A settled card states its outcome; the pitch for an action the
                user already answered would only read as stale. */}
            {verdict ? block.outcome_note : descriptor.body(params)}
          </p>
        </div>
      </div>

      <ParameterSummary rows={descriptor.summary(params)} />
      <StatusNotice block={block} />

      {verdict?.footer && VerdictIcon ? (
        <div className="flex items-start gap-2 border-t border-border bg-muted px-4 py-3">
          <VerdictIcon
            className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${verdict.iconClass}`}
          />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {verdict.footer}
          </p>
        </div>
      ) : null}

      {!verdict && !unsupported ? (
        <div className="flex items-start gap-2 px-4 py-3">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-nyx-secondary-400" />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {descriptor.assurance}
          </p>
        </div>
      ) : null}

      {!verdict ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted px-4 py-3">
          {!unsupported ? (
            <Button
              type="button"
              variant="primary"
              size="sm"
              disabled={primaryDisabled}
              onClick={() => void beginJourney()}
            >
              {busy ? <Loader2 className="animate-spin" /> : <ShieldCheck />}
              {awaitingAuthorization
                ? "Waiting for authorization"
                : busy
                  ? descriptor.busyLabel
                  : descriptor.cta(params)}
            </Button>
          ) : null}
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={secondaryDisabled}
            onClick={() => report("declined")}
          >
            <X />
            Decline
          </Button>
          {blocked ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={secondaryDisabled}
              onClick={() => report("failed")}
            >
              <AlertTriangle />
              Report failure
            </Button>
          ) : null}
          <span className="ml-auto text-[10px] text-muted-foreground">
            Nothing is shared until you finish.
          </span>
        </div>
      ) : null}

      {/* A settled card unmounts the dialog: its journey is over, and leaving
          it mounted would let a stale flow write onto a reported outcome. */}
      {!verdict &&
      !unsupported &&
      (params.variant === "catalog" || params.variant === "custom") ? (
        <AddKeyDialog
          open={dialogOpen}
          onOpenChange={setOpen}
          prefillSlug={
            params.variant === "catalog" ? params.service_slug : undefined
          }
          prefillIncludeAllCatalog={params.variant === "catalog"}
          prefillNodeId={params.via_node_id ?? undefined}
          prefillTargetOrgId={params.target_org_id ?? undefined}
          prefillCustom={
            params.variant === "custom"
              ? {
                  name: params.name,
                  endpointUrl: params.endpoint_url,
                  authMethod: params.auth_method,
                  authKeyName: params.auth_key_name,
                }
              : undefined
          }
          // Provider consent pages can never be iframed, so a top-level popup
          // is the only handoff that keeps this conversation alive underneath
          // it. Without this the action card fell back to the legacy path — a
          // `target="_blank"` link needing a second click, and a callback that
          // redirects to the key page, taking the chat tab with it.
          launch="popup"
          flow="cc"
          onPopupViewResult={() => {
            // The popup is done and the user asked to come back. Close the
            // dialog and let the card settle itself from the key's terminal
            // status — the outcome belongs in the transcript, not on a
            // detour to the keys page.
            setOpen(false);
            return true;
          }}
          onSuccess={({ userServiceId }) => {
            if (!userServiceId.trim()) {
              report("completed");
              return;
            }
            report("completed", { userService: { userServiceId } });
          }}
          // Deliberately no onAuthorizationAborted: closing this short-lived
          // dialog is not abandonment. The store-backed watch must survive
          // dismissal/remount and settle the busy card (#1384).
          onAuthorizationPending={(attempt) => {
            watchSettledRef.current = null;
            beginPendingAuth(block.block_id, {
              ...attempt,
              startedAt: Date.now(),
            });
          }}
        />
      ) : null}
      {!verdict && !unsupported && isDialogParams(params)
        ? (() => {
            const binding = dialogBindingFor(params.variant);
            return (
              <binding.Dialog
                open={dialogOpen}
                onOpenChange={setOpen}
                actionRequestId={block.action_request_id}
                params={binding.toProps(params)}
                onComplete={(id) =>
                  report("completed", descriptor.resource(id))
                }
              />
            );
          })()
        : null}
      {!verdict &&
      !unsupported &&
      params.variant === "service_reauthorize" &&
      reauthorizeKey ? (
        <AddKeyDialog
          open={dialogOpen}
          onOpenChange={setOpen}
          prefillIncludeAllCatalog
          prefillScopes={params.requested_scopes}
          reconnectKey={reauthorizeKey}
          launch="popup"
          flow="cc"
          onPopupViewResult={() => {
            setOpen(false);
            return true;
          }}
          onSuccess={({ userServiceId }) => {
            if (userServiceId !== params.user_service_id) {
              void onBlock(block.block_id, REAUTHORIZE_IDENTITY_NOTE);
              return;
            }
            // Claim the card before awaiting. `report` used to run
            // synchronously here, which is what stopped the watch from
            // settling the same attempt; the scope read opens a gap the watch
            // could otherwise resolve into a second report.
            watchSettledRef.current = authorizationAttemptId;
            resolvingRef.current = true;
            // Same scope confirmation as the watch path: the dialog reporting
            // success only means the handshake finished.
            void scopeGrantShortfall(
              params.user_service_id,
              params.requested_scopes,
            )
              .then((shortfall) => {
                if (shortfall) {
                  resolvingRef.current = false;
                  return onBlock(block.block_id, shortfall);
                }
                report("completed", {
                  userService: { userServiceId: params.user_service_id },
                });
                return undefined;
              })
              .catch(() => {
                resolvingRef.current = false;
                void onBlock(
                  block.block_id,
                  REAUTHORIZE_EVIDENCE_UNREADABLE_NOTE,
                );
              });
          }}
          onAuthorizationPending={(attempt) => {
            if (attempt.keyId !== params.user_service_id) {
              void onBlock(block.block_id, REAUTHORIZE_IDENTITY_NOTE);
              return;
            }
            watchSettledRef.current = null;
            beginPendingAuth(block.block_id, {
              ...attempt,
              startedAt: Date.now(),
            });
          }}
        />
      ) : null}
    </section>
  );
}
