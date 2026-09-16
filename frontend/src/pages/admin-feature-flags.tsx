import {
  changedFields,
  describeChanges,
  hasFieldConflicts,
} from "@/lib/form-changes";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { ChevronDown, Search, User } from "lucide-react";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { PowerButtonIcon } from "@/components/icons/empty-state";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn, formatRelativeTime } from "@/lib/utils";
import { useAdminUsers } from "@/hooks/use-admin";
import type { AdminUser } from "@/types/admin";
import {
  useAdminFeatureFlags,
  useClearAdminFeatureFlag,
  useSetAdminFeatureFlag,
  useUpdateAdminFeatureFlagMetadata,
} from "@/hooks/use-admin-feature-flags";
import {
  MAX_FEATURE_FLAG_DESCRIPTION_LENGTH,
  MAX_FEATURE_FLAG_OWNER_LENGTH,
} from "@/schemas/admin-feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

type ScopeState = "inherit" | "enabled" | "disabled";
type FlagKind = "experiment" | "entitlement" | "ops";

interface Assignment {
  readonly id: string;
  readonly label: string;
  /** True when the referenced account no longer resolves (deleted). */
  readonly missingAccount?: boolean;
  state: ScopeState;
}
interface FlagRow {
  readonly key: string;
  /** Effective description: the admin-authored one when set, else the code one. */
  readonly description: string;
  /** The code-declared description — what a reset falls back to. */
  readonly codeDescription: string;
  /** The admin-authored description, when one exists. */
  readonly customDescription: string | null;
  readonly owner: string | null;
  readonly metadataUpdatedAt: string | null;
  readonly kind: FlagKind;
  readonly defaultEnabled: boolean;
  global: ScopeState;
  orgs: Assignment[];
  users: Assignment[];
}

type ScopeType = "global" | "org" | "user";
const draftKey = (flag: string, type: ScopeType, id: string) =>
  `${flag}\u001f${type}\u001f${id}`;

export function AdminFeatureFlagsPage() {
  const { data, isLoading, error } = useAdminFeatureFlags();
  const setOverride = useSetAdminFeatureFlag();
  const clearOverride = useClearAdminFeatureFlag();
  // Operators get read access to this page but their writes 403 — render
  // them a read-only view instead of controls that fail on Apply.
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);
  const [drafts, setDrafts] = useState<Record<string, ScopeState>>({});
  const draftVersions = useRef<Record<string, number>>({});
  const draftBaselines = useRef<Record<string, ScopeState>>({});
  const [applying, setApplying] = useState(false);
  const [search, setSearch] = useState("");
  const [openKeys, setOpenKeys] = useState<string[]>([]);
  const flags: FlagRow[] = (data?.flags ?? []).map((flag) => ({
    key: flag.key,
    description: flag.description,
    codeDescription: flag.code_description ?? flag.description,
    customDescription: flag.custom_description ?? null,
    owner: flag.owner ?? null,
    metadataUpdatedAt: flag.metadata_updated_at ?? null,
    kind: flag.key.startsWith("experimental:") ? "experiment" : "ops",
    defaultEnabled: flag.default_enabled,
    global:
      flag.global_override == null
        ? "inherit"
        : flag.global_override
          ? "enabled"
          : "disabled",
    orgs: (flag.org_overrides ?? []).map((override) => ({
      id: override.org_id,
      label: orgOverrideLabel(override),
      missingAccount:
        override.org_display_name == null && override.org_slug == null,
      state: override.enabled ? "enabled" : "disabled",
    })),
    users: flag.user_overrides.map((override) => ({
      id: override.user_id,
      label: userOverrideLabel(override),
      missingAccount:
        override.user_email == null && override.user_display_name == null,
      state: override.enabled ? "enabled" : "disabled",
    })),
  }));

  const flagByKey = (key: string) => flags.find((f) => f.key === key);
  const persisted = (flag: string, type: ScopeType, id: string): ScopeState => {
    const f = flagByKey(flag);
    if (!f) return "inherit";
    if (type === "global") return f.global;
    const list = type === "org" ? f.orgs : f.users;
    return list.find((a) => a.id === id)?.state ?? "inherit";
  };
  const stateFor = (flag: string, type: ScopeType, id: string): ScopeState =>
    drafts[draftKey(flag, type, id)] ?? persisted(flag, type, id);
  const stage = (flag: string, type: ScopeType, id: string, s: ScopeState) => {
    const key = draftKey(flag, type, id);
    if (!(key in drafts))
      draftBaselines.current[key] = persisted(flag, type, id);
    draftVersions.current[key] = (draftVersions.current[key] ?? 0) + 1;
    setDrafts((prev) => ({ ...prev, [key]: s }));
  };

  const pending = Object.entries(drafts).filter(([k, v]) => {
    const [flag, type, id] = k.split("\u001f");
    return (
      v !== persisted(flag ?? "", (type ?? "global") as ScopeType, id ?? "")
    );
  });
  const pendingCount = pending.length;

  type RolloutChange = {
    key: string;
    flagKey: string;
    type: ScopeType;
    id: string;
    before: ScopeState;
    after: ScopeState;
    version: number;
  };
  const rolloutReview = useChangeReview<RolloutChange[]>(
    async (batch) => {
      setApplying(true);
      try {
        const results = await Promise.allSettled(
          batch.map(async (item) => {
            const targetKey = item.type === "global" ? null : item.id;
            if (item.after === "inherit")
              await clearOverride.mutateAsync({
                flagKey: item.flagKey,
                targetKind: item.type,
                targetKey,
              });
            else
              await setOverride.mutateAsync({
                flagKey: item.flagKey,
                body: {
                  target_kind: item.type,
                  target_key: targetKey,
                  enabled: item.after === "enabled",
                },
              });
          }),
        );
        const successful = batch.filter(
          (_, index) => results[index]?.status === "fulfilled",
        );
        for (const item of successful)
          draftBaselines.current[item.key] = item.after;
        setDrafts((current) => {
          const next = { ...current };
          for (const item of successful)
            if (draftVersions.current[item.key] === item.version)
              delete next[item.key];
          return next;
        });
        const failed = batch.filter(
          (_, index) => results[index]?.status === "rejected",
        );
        if (failed.length)
          toast.error(
            `Applied ${successful.length}; failed: ${failed.map((item) => `${item.flagKey} / ${item.type} ${item.id}`).join(", ")}. Review remaining changes before retrying.`,
          );
        else toast.success(`${successful.length} changes applied`);
      } finally {
        setApplying(false);
      }
    },
    (batch) =>
      batch.some(
        (item) =>
          !flagByKey(item.flagKey) ||
          persisted(item.flagKey, item.type, item.id) !== item.before,
      ),
  );

  function applyChanges() {
    const batch = pending.map(([key, after]): RolloutChange => {
      const [flagKey = "", type = "global", id = ""] = key.split("\u001f");
      return {
        key,
        flagKey,
        type: type as ScopeType,
        id,
        before:
          draftBaselines.current[key] ??
          persisted(flagKey, type as ScopeType, id),
        after,
        version: draftVersions.current[key] ?? 0,
      };
    });
    rolloutReview.review(
      batch,
      batch.map((item) => ({
        field: `${item.flagKey} / ${item.type === "global" ? "Global — all users" : `${item.type}: ${item.id}`}`,
        before: item.before,
        after:
          item.after === "inherit" ? "Inherit (remove override)" : item.after,
      })),
    );
  }

  const q = search.trim().toLowerCase();
  const filtered = q
    ? flags.filter(
        (f) =>
          f.key.toLowerCase().includes(q) ||
          f.description.toLowerCase().includes(q) ||
          (f.owner?.toLowerCase().includes(q) ?? false),
      )
    : flags;
  const isOpen = (key: string) => (q ? true : openKeys.includes(key));
  const toggle = (key: string) =>
    setOpenKeys((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key],
    );
  const pendingKeys = new Set(pending.map(([k]) => k));
  const dirtyFlags = new Set(pending.map(([k]) => k.split("\u001f")[0] ?? ""));

  return (
    <div className="space-y-6">
      {rolloutReview.dialog}
      {error && data && (
        <p role="alert">
          Unable to refresh feature flags. Your drafts are retained.
        </p>
      )}
      <PageHeader
        title="Feature Flags"
        description="Platform-wide feature rollout. Configure each flag globally, per organization, and per user."
      />

      {canWrite && pendingCount > 0 && (
        <div className="sticky top-2 z-10 flex items-center justify-between gap-3 rounded-lg border border-primary/40 bg-primary/10 px-3 py-2 shadow-sm backdrop-blur">
          <span className="text-[12px] font-medium">
            {pendingCount} unsaved change{pendingCount === 1 ? "" : "s"}
          </span>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              disabled={applying}
              onClick={() => setDrafts({})}
            >
              Discard
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={applying}
              onClick={applyChanges}
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

      {isLoading && !data ? (
        <div className="space-y-2" aria-label="Loading feature flags">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-24 w-full" />
          ))}
        </div>
      ) : error && !data ? (
        <FlagsEmptyState
          title="Failed to load feature flags"
          subtitle="Please try again later."
        />
      ) : flags.length === 0 ? (
        <FlagsEmptyState
          title="No feature flags"
          subtitle="Flags are declared in code. None have been defined yet."
        />
      ) : (
        <div className="space-y-2">
          {filtered.length === 0 && <p>No flags match “{search.trim()}”.</p>}
          {flags.map((flag) => (
            <div
              key={flag.key}
              hidden={!filtered.some((item) => item.key === flag.key)}
            >
              <FlagCard
                flag={flag}
                open={isOpen(flag.key)}
                dirty={dirtyFlags.has(flag.key)}
                onToggle={() => toggle(flag.key)}
                stateFor={(type, id) => stateFor(flag.key, type, id)}
                isPending={(type, id) =>
                  pendingKeys.has(draftKey(flag.key, type, id))
                }
                stage={(type, id, s) => stage(flag.key, type, id, s)}
                stagedUsers={stagedIds(drafts, flag.key, "user")}
                stagedOrgs={stagedIds(drafts, flag.key, "org")}
                canWrite={canWrite}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function FlagCard({
  flag,
  open,
  dirty,
  onToggle,
  stateFor,
  isPending,
  stage,
  stagedOrgs,
  stagedUsers,
  canWrite,
}: {
  readonly flag: FlagRow;
  readonly open: boolean;
  readonly dirty: boolean;
  readonly onToggle: () => void;
  readonly stateFor: (type: ScopeType, id: string) => ScopeState;
  readonly isPending: (type: ScopeType, id: string) => boolean;
  readonly stage: (type: ScopeType, id: string, s: ScopeState) => void;
  readonly stagedOrgs: readonly string[];
  readonly stagedUsers: readonly string[];
  readonly canWrite: boolean;
}) {
  const [stagedOrgLabels, setStagedOrgLabels] = useState<
    Record<string, string>
  >({});
  const [stagedUserLabels, setStagedUserLabels] = useState<
    Record<string, string>
  >({});

  const orgIds = uniq([...flag.orgs.map((o) => o.id), ...stagedOrgs]);
  const userIds = uniq([...flag.users.map((u) => u.id), ...stagedUsers]);
  const labelFor = (type: ScopeType, id: string) =>
    type === "user"
      ? (flag.users.find((user) => user.id === id)?.label ??
        stagedUserLabels[id] ??
        id)
      : type === "org"
        ? (flag.orgs.find((org) => org.id === id)?.label ??
          stagedOrgLabels[id] ??
          id)
        : id;

  // Collapsed summary pills: Global + configured orgs/users (non-inherit).
  const pills: {
    id: string;
    label: string;
    state: ScopeState;
    pending: boolean;
  }[] = [
    {
      id: "global",
      label: "Global",
      state: stateFor("global", ""),
      pending: isPending("global", ""),
    },
    ...orgIds.map((id) => ({
      id: `org-${id}`,
      label: labelFor("org", id),
      state: stateFor("org", id),
      pending: isPending("org", id),
    })),
    ...userIds.map((id) => ({
      id: `user-${id}`,
      label: labelFor("user", id),
      state: stateFor("user", id),
      pending: isPending("user", id),
    })),
  ].filter((pill) => pill.id === "global" || pill.state !== "inherit");
  const shownPills = pills.slice(0, 6);
  const extraPills = pills.length - shownPills.length;

  return (
    <Card className={cn("overflow-hidden", dirty && "ring-1 ring-primary/60")}>
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
            <Badge variant={kindVariant(flag.kind)} className="text-[11px]">
              {flag.kind}
            </Badge>
            {flag.owner && (
              <Badge
                variant="secondary"
                className="max-w-[200px] gap-1 text-[11px] font-normal"
                title={`Owner: ${flag.owner}`}
              >
                <User className="h-3 w-3 shrink-0" aria-hidden />
                <span className="min-w-0 truncate">{flag.owner}</span>
              </Badge>
            )}
          </div>
          <p className="truncate text-xs text-muted-foreground">
            {flag.description}
          </p>
          {!open && (
            <div className="flex flex-wrap items-center gap-1 pt-0.5">
              {shownPills.map((pill) => (
                <SummaryPill key={pill.id} pill={pill} />
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
            Default: {flag.defaultEnabled ? "Enabled" : "Disabled"}
          </Badge>
          <ChevronDown
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform",
              open && "rotate-180",
            )}
          />
        </div>
      </button>

      {
        <div
          hidden={!open}
          className="space-y-4 border-t border-border/60 bg-muted/20 px-3 py-3"
        >
          <Group label="Documentation">
            <FlagMetadataEditor flag={flag} canWrite={canWrite} />
          </Group>

          <Group label="Global">
            <ScopeRow
              label="All users (rollout / killswitch)"
              state={stateFor("global", "")}
              pending={isPending("global", "")}
              disabled={!canWrite}
              onChange={(s) => stage("global", "", s)}
            />
          </Group>

          <Group label="By organization">
            {canWrite && (
              <AccountSearchPicker
                kind="org"
                excludedIds={orgIds}
                onPick={(org) => {
                  setStagedOrgLabels((labels) => ({
                    ...labels,
                    [org.id]: org.label,
                  }));
                  stage("org", org.id, "enabled");
                }}
              />
            )}
            {orgIds.map((id) => {
              const assignment = flag.orgs.find((org) => org.id === id);
              return (
                <div
                  key={id}
                  title={assignment?.missingAccount ? id : undefined}
                >
                  <ScopeRow
                    label={labelFor("org", id)}
                    state={stateFor("org", id)}
                    pending={isPending("org", id)}
                    disabled={!canWrite}
                    onChange={(s) => stage("org", id, s)}
                  />
                </div>
              );
            })}
          </Group>

          <Group label="By user">
            {canWrite && (
              <AccountSearchPicker
                kind="person"
                excludedIds={userIds}
                onPick={(user) => {
                  setStagedUserLabels((labels) => ({
                    ...labels,
                    [user.id]: user.label,
                  }));
                  stage("user", user.id, "enabled");
                }}
              />
            )}
            {userIds.map((id) => {
              const assignment = flag.users.find((user) => user.id === id);
              const label = labelFor("user", id);
              return (
                <div
                  key={id}
                  title={assignment?.missingAccount ? id : undefined}
                >
                  <ScopeRow
                    label={label}
                    state={stateFor("user", id)}
                    pending={isPending("user", id)}
                    disabled={!canWrite}
                    onChange={(state) => stage("user", id, state)}
                  />
                </div>
              );
            })}
          </Group>
        </div>
      }
    </Card>
  );
}

/**
 * Inline editor for a flag's admin-authored description and owner.
 *
 * Pristine editors follow refreshed metadata, including before first expansion.
 * Once editing begins, refetches preserve the draft. Saving sends only changed fields;
 * a blank field explicitly clears that side of the metadata (description falls back to
 * the code-declared text).
 */
function FlagMetadataEditor({
  flag,
  canWrite,
}: {
  readonly flag: FlagRow;
  readonly canWrite: boolean;
}) {
  const update = useUpdateAdminFeatureFlagMetadata();
  const [description, setDescription] = useState(flag.customDescription ?? "");
  const [owner, setOwner] = useState(flag.owner ?? "");

  const current = {
    description: flag.customDescription ?? null,
    owner: flag.owner ?? null,
  };
  const [baseline, setBaseline] = useState(current);
  const [observed, setObserved] = useState(current);
  const [hydrationPending, setHydrationPending] = useState(false);
  const next = {
    description: description.trim() || null,
    owner: owner.trim() || null,
  };
  const patch = changedFields(baseline, next);
  const dirty = Object.keys(patch).length > 0;
  const stale = hasFieldConflicts(baseline, current, patch);
  const review = useChangeReview<{
    flagKey: string;
    body: Partial<typeof next>;
    before: typeof next;
  }>(
    async ({ flagKey, body }) => {
      const saved = await update.mutateAsync({ flagKey, body });
      const values = {
        description: saved.custom_description,
        owner: saved.owner,
      };
      setBaseline(values);
      setDescription(values.description ?? "");
      setOwner(values.owner ?? "");
      setHydrationPending(false);
      toast.success("Flag details saved");
    },
    (pending) => hasFieldConflicts(pending.before, current, pending.body),
    flag.key,
  );

  const sourceChanged =
    observed.description !== current.description ||
    observed.owner !== current.owner;
  if (sourceChanged) setObserved(current);
  const pristine =
    description === (baseline.description ?? "") &&
    owner === (baseline.owner ?? "");
  if (
    (sourceChanged || hydrationPending) &&
    pristine &&
    !review.saving &&
    !update.isPending
  ) {
    setBaseline(current);
    setDescription(current.description ?? "");
    setOwner(current.owner ?? "");
    setHydrationPending(false);
  } else if (sourceChanged) {
    setHydrationPending(true);
  }

  function save() {
    review.review(
      { flagKey: flag.key, body: patch, before: baseline },
      describeChanges(baseline, patch),
    );
  }

  if (!canWrite) {
    return (
      <dl className="space-y-2 px-3 py-2.5 text-[12px]">
        <div className="space-y-0.5">
          <dt className="text-[11px] text-muted-foreground">Description</dt>
          <dd className="text-foreground">{flag.description}</dd>
        </div>
        <div className="space-y-0.5">
          <dt className="text-[11px] text-muted-foreground">Owner</dt>
          <dd className={cn(!flag.owner && "text-muted-foreground")}>
            {flag.owner ?? "Unassigned"}
          </dd>
        </div>
      </dl>
    );
  }

  return (
    <div className="space-y-2.5 px-3 py-2.5">
      {review.dialog}
      {stale && (
        <StaleFormNotice
          onReload={() => {
            setBaseline(current);
            setDescription(current.description ?? "");
            setOwner(current.owner ?? "");
            setHydrationPending(false);
            review.cancel();
          }}
        />
      )}
      <div className="space-y-1">
        <Label
          htmlFor={`flag-description-${flag.key}`}
          className="text-[11px] font-normal"
        >
          Description — what this flag controls
        </Label>
        <Input
          id={`flag-description-${flag.key}`}
          disabled={review.saving || update.isPending}
          value={description}
          maxLength={MAX_FEATURE_FLAG_DESCRIPTION_LENGTH}
          onChange={(event) => setDescription(event.target.value)}
          placeholder={flag.codeDescription}
          className="h-8 text-[12px]"
        />
      </div>
      <div className="space-y-1">
        <Label
          htmlFor={`flag-owner-${flag.key}`}
          className="text-[11px] font-normal"
        >
          Owner — who to ask about it
        </Label>
        <Input
          id={`flag-owner-${flag.key}`}
          disabled={review.saving || update.isPending}
          value={owner}
          maxLength={MAX_FEATURE_FLAG_OWNER_LENGTH}
          onChange={(event) => setOwner(event.target.value)}
          placeholder="Unassigned — e.g. Platform team"
          className="h-8 text-[12px]"
        />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-[11px] text-muted-foreground">
          {flag.customDescription == null
            ? "Using the description declared in code."
            : flag.metadataUpdatedAt
              ? `Edited ${formatRelativeTime(flag.metadataUpdatedAt)}.`
              : "Edited by an admin."}
        </p>
        <div className="flex items-center gap-2">
          {description !== "" && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDescription("")}
              disabled={update.isPending}
            >
              Use code default
            </Button>
          )}
          <Button
            variant="primary"
            size="sm"
            onClick={save}
            disabled={stale || !dirty || update.isPending}
          >
            {update.isPending ? "Saving…" : "Save details"}
          </Button>
        </div>
      </div>
    </div>
  );
}

interface SummaryPillValue {
  readonly label: string;
  readonly state: ScopeState;
  readonly pending: boolean;
}

function SummaryPill({ pill }: { readonly pill: SummaryPillValue }) {
  return (
    <Badge
      variant={pill.state === "enabled" ? "success" : "secondary"}
      className={cn(
        "flex max-w-[180px] gap-1 text-[11px] font-normal",
        pill.state === "inherit" && !pill.pending && "opacity-60",
        pill.pending && "ring-1 ring-primary/70",
      )}
      title={`${pill.label} · ${word(pill.state)}`}
    >
      <span className="min-w-0 truncate">{pill.label}</span>
      <span className="shrink-0">· {word(pill.state)}</span>
    </Badge>
  );
}

/** Default suggestions shown on focus before the admin types anything. */
const DEFAULT_SUGGESTION_COUNT = 5;
/** Over-fetch so exclusions don't leave the suggestion list short. */
const DEFAULT_SUGGESTION_FETCH = 8;

function AccountSearchPicker({
  kind,
  excludedIds,
  onPick,
}: {
  readonly kind: "person" | "org";
  readonly excludedIds: readonly string[];
  readonly onPick: (picked: { id: string; label: string }) => void;
}) {
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [focused, setFocused] = useState(false);
  const normalizedSearch = search.trim();

  useEffect(() => {
    const timer = window.setTimeout(
      () => setDebouncedSearch(normalizedSearch),
      300,
    );
    return () => window.clearTimeout(timer);
  }, [normalizedSearch]);

  const { data, isLoading } = useAdminUsers(
    1,
    20,
    debouncedSearch || undefined,
    kind,
  );
  const waitingForSearch = normalizedSearch !== debouncedSearch;
  const results =
    debouncedSearch && !waitingForSearch
      ? (data?.users ?? []).filter(
          (account) => !excludedIds.includes(account.id),
        )
      : [];

  // A flag card renders one picker per scope, so only fetch defaults while this
  // picker's dropdown is actually open; identical keys dedupe across cards.
  const showDefaults = focused && !normalizedSearch;
  const { data: defaultsData, isLoading: defaultsLoading } = useAdminUsers(
    1,
    DEFAULT_SUGGESTION_FETCH,
    undefined,
    kind,
    { enabled: showDefaults },
  );
  const defaults = (defaultsData?.users ?? [])
    .filter((account) => !excludedIds.includes(account.id))
    .slice(0, DEFAULT_SUGGESTION_COUNT);
  const remaining = Math.max(0, (defaultsData?.total ?? 0) - defaults.length);

  const isOrg = kind === "org";
  const pick = (account: AdminUser) => {
    onPick({ id: account.id, label: accountLabel(account, isOrg) });
    setSearch("");
    setDebouncedSearch("");
  };

  return (
    <div
      className="space-y-2 border-b border-border/40 px-3 py-2 last:border-b-0"
      onFocus={() => setFocused(true)}
      onBlur={(event) => {
        // Keep the dropdown open while focus moves within the picker, so both
        // tabbing to a suggestion and clicking one still land a pick.
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setFocused(false);
        }
      }}
    >
      <div className="relative">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={
            isOrg ? "Search organizations by name…" : "Search users by email…"
          }
          aria-label={
            isOrg ? "Search organizations by name" : "Search users by email"
          }
          className="h-8 pl-8 text-[12px]"
        />
      </div>
      {normalizedSearch ? (
        <div className="max-h-40 overflow-y-auto rounded-md border border-border/60">
          {isLoading || waitingForSearch ? (
            <p className="px-3 py-2 text-xs text-muted-foreground">
              Searching…
            </p>
          ) : results.length === 0 ? (
            <p className="px-3 py-2 text-xs text-muted-foreground">
              {isOrg
                ? "No organizations match this search."
                : "No users match this search."}
            </p>
          ) : (
            results.map((account) => (
              <AccountOption
                key={account.id}
                account={account}
                isOrg={isOrg}
                onPick={pick}
              />
            ))
          )}
        </div>
      ) : (
        showDefaults && (
          <div className="max-h-40 overflow-y-auto rounded-md border border-border/60">
            {defaultsLoading ? (
              <p className="px-3 py-2 text-xs text-muted-foreground">
                Loading…
              </p>
            ) : defaults.length === 0 ? (
              <p className="px-3 py-2 text-xs text-muted-foreground">
                {isOrg
                  ? "No organizations to suggest."
                  : "No users to suggest."}
              </p>
            ) : (
              <>
                {defaults.map((account) => (
                  <AccountOption
                    key={account.id}
                    account={account}
                    isOrg={isOrg}
                    onPick={pick}
                  />
                ))}
                {remaining > 0 && (
                  <p className="border-t border-border/40 px-3 py-2 text-[11px] text-muted-foreground">
                    +{remaining} more — keep typing to narrow
                  </p>
                )}
              </>
            )}
          </div>
        )
      )}
    </div>
  );
}

function AccountOption({
  account,
  isOrg,
  onPick,
}: {
  readonly account: AdminUser;
  readonly isOrg: boolean;
  readonly onPick: (account: AdminUser) => void;
}) {
  return (
    <button
      type="button"
      // Suppress the input's blur so the click lands before the dropdown closes.
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => onPick(account)}
      className="block w-full border-b border-border/40 px-3 py-2 text-left text-xs last:border-b-0 hover:bg-muted/50"
    >
      <span className="block truncate font-medium">
        {accountLabel(account, isOrg)}
      </span>
      <span className="block truncate text-muted-foreground">
        {isOrg ? (account.slug ?? account.id) : account.id}
      </span>
    </button>
  );
}

function accountLabel(account: AdminUser, isOrg: boolean): string {
  return isOrg
    ? (account.display_name ?? account.slug ?? account.id)
    : account.email;
}

function FlagsEmptyState({
  title,
  subtitle,
}: {
  readonly title: string;
  readonly subtitle: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
      <PowerButtonIcon className="h-64 w-64 text-muted-foreground" />
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-muted-foreground">{title}</p>
        <p className="text-xs text-muted-foreground">{subtitle}</p>
      </div>
    </div>
  );
}

function Group({
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
  pending,
  disabled = false,
  onChange,
}: {
  readonly label: string;
  readonly state: ScopeState;
  readonly pending: boolean;
  readonly disabled?: boolean;
  readonly onChange: (s: ScopeState) => void;
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
        disabled={disabled}
        onValueChange={(s) => onChange(s as ScopeState)}
      >
        <SelectTrigger className="h-7 w-[120px] text-[12px]" aria-label={label}>
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

// ── helpers ─────────────────────────────────────────────────────────────────
function word(s: ScopeState): string {
  return s === "enabled"
    ? "Enabled"
    : s === "disabled"
      ? "Disabled"
      : "Inherit";
}
function kindVariant(kind: FlagKind): "info" | "accent" | "secondary" {
  return kind === "experiment"
    ? "info"
    : kind === "entitlement"
      ? "accent"
      : "secondary";
}
function uniq(ids: string[]): string[] {
  return Array.from(new Set(ids));
}
function stagedIds(
  drafts: Record<string, ScopeState>,
  flag: string,
  type: ScopeType,
): string[] {
  return Object.keys(drafts)
    .map((k) => k.split("\u001f"))
    .filter((p) => p[0] === flag && p[1] === type && p[2])
    .map((p) => p[2] as string);
}

function userOverrideLabel(override: {
  readonly user_id: string;
  readonly user_email: string | null;
  readonly user_display_name: string | null;
}): string {
  const displayName = override.user_display_name?.trim();
  if (displayName && override.user_email) {
    return `${displayName} (${override.user_email})`;
  }
  return displayName || override.user_email || override.user_id;
}

function orgOverrideLabel(override: {
  readonly org_id: string;
  readonly org_display_name: string | null;
  readonly org_slug: string | null;
}): string {
  const displayName = override.org_display_name?.trim();
  if (displayName && override.org_slug) {
    return `${displayName} (${override.org_slug})`;
  }
  return displayName || override.org_slug || override.org_id;
}
