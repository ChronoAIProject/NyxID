import { type ReactNode, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { PowerButtonIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ApiError } from "@/lib/api-client";
import { cn } from "@/lib/utils";
import {
  useClearOrgFeatureFlag,
  useOrgFeatureFlags,
  useSetOrgFeatureFlag,
} from "@/hooks/use-org-feature-flags";
import { useOrgMembers } from "@/hooks/use-org-members";
import type {
  FeatureFlagItem,
  FlagTargetKind,
} from "@/schemas/feature-flags";
import type { MemberResponse } from "@/schemas/orgs";

// Roles the backend supports for role-scoped overrides (OrgRole).
const OVERRIDE_ROLES = ["admin", "member", "viewer"] as const;

// How many override pills to show on a collapsed row before "+N more".
const MAX_SUMMARY_PILLS = 6;

/** Tri-state per scope: fall through to the broader layer, or force on/off. */
type ScopeState = "inherit" | "enabled" | "disabled";

// Encodes (flag, target) into a stable draft-map key. Unit separator can't
// appear in flag keys, role names, or member UUIDs.
const SEP = "\u001f";
const scopeKey = (
  flagKey: string,
  kind: FlagTargetKind,
  value: string | null,
): string => `${flagKey}${SEP}${kind}${SEP}${value ?? ""}`;

function decodeScopeKey(key: string): {
  flagKey: string;
  kind: FlagTargetKind;
  value: string | null;
} {
  const parts = key.split(SEP);
  const raw = parts[2] ?? "";
  return {
    flagKey: parts[0] ?? "",
    kind: (parts[1] ?? "org") as FlagTargetKind,
    value: raw === "" ? null : raw,
  };
}

function persistedState(
  flag: FeatureFlagItem | undefined,
  kind: FlagTargetKind,
  value: string | null,
): ScopeState {
  const o = flag?.overrides.find(
    (x) => x.target_kind === kind && (x.target_value ?? null) === value,
  );
  return o ? (o.enabled ? "enabled" : "disabled") : "inherit";
}

/**
 * Org-admin surface to toggle code-declared feature flags for this org,
 * scoped org-wide, per role, or per member. Precedence (most specific wins):
 * user > role > org > code default.
 *
 * Edits are staged locally; nothing hits the server until "Apply changes" is
 * clicked. Flags are searchable and shown as collapsible rows.
 */
export function OrgFeatureFlagsPanel({ orgId }: { readonly orgId: string }) {
  const { data, isLoading } = useOrgFeatureFlags(orgId);
  const { data: members } = useOrgMembers(orgId);
  const setMutation = useSetOrgFeatureFlag(orgId);
  const clearMutation = useClearOrgFeatureFlag(orgId);
  const [search, setSearch] = useState("");
  const [openKeys, setOpenKeys] = useState<string[]>([]);
  // Pending edits: scopeKey -> desired state. Empty until the user changes a
  // dropdown; cleared on Apply / Discard.
  const [drafts, setDrafts] = useState<Record<string, ScopeState>>({});

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  const flags = data?.flags ?? [];
  const flagByKey = (key: string) => flags.find((f) => f.key === key);

  const persistedFor = (
    flagKey: string,
    kind: FlagTargetKind,
    value: string | null,
  ) => persistedState(flagByKey(flagKey), kind, value);
  // Displayed state = staged edit if any, else what's on the server.
  const stateFor = (
    flagKey: string,
    kind: FlagTargetKind,
    value: string | null,
  ): ScopeState =>
    drafts[scopeKey(flagKey, kind, value)] ?? persistedFor(flagKey, kind, value);
  const stage = (
    flagKey: string,
    kind: FlagTargetKind,
    value: string | null,
    state: ScopeState,
  ) =>
    setDrafts((prev) => ({
      ...prev,
      [scopeKey(flagKey, kind, value)]: state,
    }));

  // Only drafts that actually differ from the server count as pending.
  const pending = Object.entries(drafts).filter(([k, v]) => {
    const { flagKey, kind, value } = decodeScopeKey(k);
    return v !== persistedFor(flagKey, kind, value);
  });
  const pendingCount = pending.length;
  const applying = setMutation.isPending || clearMutation.isPending;

  // Which flags / scopes have unsaved edits, for highlighting.
  const dirtyFlagKeys = new Set(pending.map(([k]) => decodeScopeKey(k).flagKey));
  const pendingKeys = new Set(pending.map(([k]) => k));
  const isScopePending = (
    flagKey: string,
    kind: FlagTargetKind,
    value: string | null,
  ) => pendingKeys.has(scopeKey(flagKey, kind, value));

  // Member ids that have a staged (not-yet-saved) row for a flag, so a newly
  // added member override renders before it's applied.
  const stagedUserIds = (flagKey: string): string[] =>
    Object.keys(drafts)
      .map(decodeScopeKey)
      .filter((d) => d.flagKey === flagKey && d.kind === "user" && d.value)
      .map((d) => d.value as string);

  async function applyChanges() {
    try {
      await Promise.all(
        pending.map(([k, state]) => {
          const { flagKey, kind, value } = decodeScopeKey(k);
          if (state === "inherit") {
            return clearMutation.mutateAsync({
              flagKey,
              targetKind: kind,
              targetValue: value,
            });
          }
          return setMutation.mutateAsync({
            flagKey,
            body: {
              target_kind: kind,
              target_value: value,
              enabled: state === "enabled",
            },
          });
        }),
      );
      setDrafts({});
      toast.success(
        `${pendingCount} change${pendingCount === 1 ? "" : "s"} applied`,
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to apply changes",
      );
    }
  }

  const q = search.trim().toLowerCase();
  const filtered = q
    ? flags.filter(
        (f) =>
          f.key.toLowerCase().includes(q) ||
          f.description.toLowerCase().includes(q),
      )
    : flags;
  const isOpen = (key: string) => (q ? true : openKeys.includes(key));
  const toggle = (key: string) =>
    setOpenKeys((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key],
    );

  return (
    <div className="space-y-3">
      {pendingCount > 0 && (
        <div className="sticky top-2 z-10 flex items-center justify-between gap-3 rounded-lg border border-primary/40 bg-primary/10 px-3 py-2 shadow-sm backdrop-blur">
          <span className="text-[12px] font-medium text-foreground">
            {pendingCount} unsaved change{pendingCount === 1 ? "" : "s"}
          </span>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDrafts({})}
              disabled={applying}
            >
              Discard
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={() => void applyChanges()}
              isLoading={applying}
            >
              Apply changes
            </Button>
          </div>
        </div>
      )}

      <div className="relative">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search feature flags…"
          className="h-9 pl-8"
          aria-label="Search feature flags"
        />
      </div>

      {flags.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <PowerButtonIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">
              No feature flags
            </p>
            <p className="text-xs text-muted-foreground">
              No features are available for this organization yet.
            </p>
          </div>
        </div>
      ) : filtered.length === 0 ? (
        <Card>
          <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
            No flags match “{search.trim()}”.
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-2">
          {filtered.map((flag) => (
            <FeatureFlagCard
              key={flag.key}
              flag={flag}
              members={members ?? []}
              open={isOpen(flag.key)}
              busy={applying}
              dirty={dirtyFlagKeys.has(flag.key)}
              onToggle={() => toggle(flag.key)}
              stateFor={(kind, value) => stateFor(flag.key, kind, value)}
              isPending={(kind, value) => isScopePending(flag.key, kind, value)}
              stage={(kind, value, state) => stage(flag.key, kind, value, state)}
              stagedUserIds={stagedUserIds(flag.key)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function FeatureFlagCard({
  flag,
  members,
  open,
  busy,
  dirty,
  onToggle,
  stateFor,
  isPending,
  stage,
  stagedUserIds,
}: {
  readonly flag: FeatureFlagItem;
  readonly members: readonly MemberResponse[];
  readonly open: boolean;
  readonly busy: boolean;
  readonly dirty: boolean;
  readonly onToggle: () => void;
  readonly stateFor: (kind: FlagTargetKind, value: string | null) => ScopeState;
  readonly isPending: (kind: FlagTargetKind, value: string | null) => boolean;
  readonly stage: (
    kind: FlagTargetKind,
    value: string | null,
    state: ScopeState,
  ) => void;
  readonly stagedUserIds: readonly string[];
}) {
  const [addUserId, setAddUserId] = useState<string>("");
  const disabled = !flag.org_manageable;
  const locked = disabled || busy;

  const activeMembers = members.filter((m) => m.revoked_at === null);
  const memberLabel = (userId: string): string => {
    const m = activeMembers.find((mem) => mem.user_id === userId);
    return m?.email ?? m?.display_name ?? userId;
  };

  // Member rows to show = saved user overrides ∪ staged additions.
  const persistedUserIds = flag.overrides
    .filter((o) => o.target_kind === "user")
    .map((o) => o.target_value)
    .filter((v): v is string => Boolean(v));
  const userIds = Array.from(new Set([...persistedUserIds, ...stagedUserIds]));
  const addable = activeMembers.filter((m) => !userIds.includes(m.user_id));
  const addPlaceholder =
    addable.length > 0
      ? "Add member override…"
      : activeMembers.length === 0
        ? "No members in this org"
        : "All members already have an override";

  // Summary pills: org + every role always (inherit included), plus configured
  // members (a member is "configured" when its effective state isn't inherit).
  const summaryPills: {
    id: string;
    label: string;
    state: ScopeState;
    pending: boolean;
  }[] = [
    { id: "org", label: "Org", state: stateFor("org", null), pending: isPending("org", null) },
    ...OVERRIDE_ROLES.map((role) => ({
      id: `role-${role}`,
      label: roleTitle(role),
      state: stateFor("role", role),
      pending: isPending("role", role),
    })),
    ...userIds
      .map((uid) => ({
        id: `user-${uid}`,
        label: memberLabel(uid),
        state: stateFor("user", uid),
        pending: isPending("user", uid),
      }))
      .filter((p) => p.state !== "inherit"),
  ];
  const shownPills = summaryPills.slice(0, MAX_SUMMARY_PILLS);
  const extraPills = summaryPills.length - shownPills.length;

  return (
    <Card
      className={cn(
        "overflow-hidden",
        dirty && "ring-1 ring-primary/60",
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center justify-between gap-3 p-3 text-left transition-colors hover:bg-muted/30"
      >
        <div className="min-w-0 space-y-1">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-foreground">
              {flag.key}
            </span>
            {disabled && (
              <Badge variant="info" className="text-[11px]">
                Managed by NyxID
              </Badge>
            )}
          </div>
          <p className="truncate text-xs text-muted-foreground">
            {flag.description}
          </p>
          {!open && (
            <div className="flex flex-wrap items-center gap-1 pt-0.5">
              {shownPills.map((p) => (
                <Badge
                  key={p.id}
                  variant={p.state === "enabled" ? "success" : "secondary"}
                  className={cn(
                    "flex max-w-[180px] gap-1 text-[11px] font-normal",
                    p.state === "inherit" && !p.pending && "opacity-60",
                    p.pending && "ring-1 ring-primary/70",
                  )}
                  title={`${p.label} · ${stateWord(p.state)}${p.pending ? " (unsaved)" : ""}`}
                >
                  <span className="min-w-0 truncate">{p.label}</span>
                  <span className="shrink-0">· {stateWord(p.state)}</span>
                </Badge>
              ))}
              {extraPills > 0 && (
                <Badge
                  variant="secondary"
                  className="text-[11px] font-normal text-muted-foreground"
                >
                  +{extraPills} more
                </Badge>
              )}
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-3">
          <Badge variant="secondary" className="text-[11px]">
            Default: {flag.default_enabled ? "Enabled" : "Disabled"}
          </Badge>
          <ChevronDown
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform",
              open && "rotate-180",
            )}
          />
        </div>
      </button>

      {open && (
        <div className="space-y-4 border-t border-border/60 bg-muted/20 px-3 py-3">
          <FlagGroup label="Organization">
            <ScopeRow
              label="Organization-wide"
              state={stateFor("org", null)}
              disabled={locked}
              pending={isPending("org", null)}
              onChange={(state) => stage("org", null, state)}
            />
          </FlagGroup>

          <FlagGroup label="By role">
            {OVERRIDE_ROLES.map((role) => (
              <ScopeRow
                key={role}
                label={roleTitle(role)}
                state={stateFor("role", role)}
                disabled={locked}
                pending={isPending("role", role)}
                onChange={(state) => stage("role", role, state)}
              />
            ))}
          </FlagGroup>

          <FlagGroup label="By member">
            {!disabled && (
              <div className="border-b border-border/40 px-3 py-2 last:border-b-0">
                <Select
                  value={addUserId}
                  onValueChange={(userId) => {
                    setAddUserId("");
                    stage("user", userId, "enabled");
                  }}
                  disabled={busy || addable.length === 0}
                >
                  <SelectTrigger className="h-8 w-full text-[12px]">
                    <SelectValue placeholder={addPlaceholder} />
                  </SelectTrigger>
                  <SelectContent>
                    {addable.map((m) => (
                      <SelectItem key={m.user_id} value={m.user_id}>
                        {m.email ?? m.display_name ?? m.user_id}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
            {userIds.map((uid) => (
              <ScopeRow
                key={uid}
                label={memberLabel(uid)}
                state={stateFor("user", uid)}
                disabled={locked}
                pending={isPending("user", uid)}
                onChange={(state) => stage("user", uid, state)}
              />
            ))}
            {disabled && userIds.length === 0 && (
              <p className="px-3 py-2 text-xs text-muted-foreground">
                No member-specific overrides.
              </p>
            )}
          </FlagGroup>
        </div>
      )}
    </Card>
  );
}

/** A labeled, bordered group of scope rows — makes the org / role / member
 * sections visually distinct. */
function FlagGroup({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {label}
      </p>
      <div className="overflow-hidden rounded-lg border border-border/60 bg-background">
        {children}
      </div>
    </div>
  );
}

function ScopeRow({
  label,
  state,
  disabled,
  pending = false,
  onChange,
}: {
  readonly label: string;
  readonly state: ScopeState;
  readonly disabled: boolean;
  readonly pending?: boolean;
  readonly onChange: (state: ScopeState) => void;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-between gap-3 px-3 py-1.5",
        pending && "bg-primary/[0.07]",
      )}
    >
      <span className="flex items-center gap-1.5 text-[12px] font-medium text-foreground">
        {/* Dot slot is always reserved so labels don't shift when a row
            becomes pending; only its color/visibility changes. */}
        <span
          aria-hidden
          className={cn(
            "h-1.5 w-1.5 shrink-0 rounded-full bg-primary",
            !pending && "invisible",
          )}
        />
        {label}
      </span>
      <Select
        value={state}
        onValueChange={(next) => onChange(next as ScopeState)}
        disabled={disabled}
      >
        <SelectTrigger
          className="h-7 w-[120px] text-[12px]"
          aria-label={label}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="inherit">Inherit</SelectItem>
          <SelectItem value="enabled">Enabled</SelectItem>
          <SelectItem value="disabled">Disabled</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

function roleTitle(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

/** Exact dropdown labels, reused for the summary pills. */
function stateWord(state: ScopeState): string {
  return state === "enabled"
    ? "Enabled"
    : state === "disabled"
      ? "Disabled"
      : "Inherit";
}
