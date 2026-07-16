import { type ReactNode, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { PowerButtonIcon } from "@/components/icons/empty-state";
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
import { cn } from "@/lib/utils";
import { useAdminUsers } from "@/hooks/use-admin";
import {
  useAdminFeatureFlags,
  useClearAdminFeatureFlag,
  useSetAdminFeatureFlag,
} from "@/hooks/use-admin-feature-flags";

type ScopeState = "inherit" | "enabled" | "disabled";
type FlagKind = "experiment" | "entitlement" | "ops";

interface Assignment {
  readonly id: string;
  readonly label: string;
  state: ScopeState;
}
interface FlagRow {
  readonly key: string;
  readonly description: string;
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
  const { data: usersData, isLoading: usersLoading } = useAdminUsers(1, 100);
  const setOverride = useSetAdminFeatureFlag();
  const clearOverride = useClearAdminFeatureFlag();
  const [drafts, setDrafts] = useState<Record<string, ScopeState>>({});
  const [search, setSearch] = useState("");
  const [openKeys, setOpenKeys] = useState<string[]>([]);
  const userOptions = (usersData?.users ?? []).map((user) => ({
    id: user.id,
    label: user.email,
  }));
  const userLabels = new Map(userOptions.map((user) => [user.id, user.label]));
  const flags: FlagRow[] = (data?.flags ?? []).map((flag) => ({
    key: flag.key,
    description: flag.description,
    kind: flag.key.startsWith("experimental:") ? "experiment" : "ops",
    defaultEnabled: flag.default_enabled,
    global:
      flag.global_override == null
        ? "inherit"
        : flag.global_override
          ? "enabled"
          : "disabled",
    orgs: [],
    users: flag.user_overrides.map((override) => ({
      id: override.user_id,
      label: userLabels.get(override.user_id) ?? override.user_id,
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
  const stage = (flag: string, type: ScopeType, id: string, s: ScopeState) =>
    setDrafts((prev) => ({ ...prev, [draftKey(flag, type, id)]: s }));

  const pending = Object.entries(drafts).filter(([k, v]) => {
    const [flag, type, id] = k.split("\u001f");
    return v !== persisted(flag ?? "", (type ?? "global") as ScopeType, id ?? "");
  });
  const pendingCount = pending.length;

  async function applyChanges() {
    try {
      await Promise.all(
        pending.map(async ([key, scopeState]) => {
          const [flagKey = "", type = "global", id = ""] = key.split("\u001f");
          const targetKind = type === "user" ? "user" : "global";
          const targetKey = targetKind === "user" ? id : null;
          if (scopeState === "inherit") {
            await clearOverride.mutateAsync({ flagKey, targetKind, targetKey });
          } else {
            await setOverride.mutateAsync({
              flagKey,
              body: {
                target_kind: targetKind,
                target_key: targetKey,
                enabled: scopeState === "enabled",
              },
            });
          }
        }),
      );
      setDrafts({});
      toast.success(
        `${pendingCount} change${pendingCount === 1 ? "" : "s"} applied`,
      );
    } catch {
      toast.error("Failed to apply feature flag changes");
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
  const pendingKeys = new Set(pending.map(([k]) => k));
  const dirtyFlags = new Set(
    pending.map(([k]) => k.split("\u001f")[0] ?? ""),
  );

  return (
    <div className="space-y-6">
      <PageHeader
        title="Feature Flags"
        description="Platform-wide feature rollout. Configure each flag globally, per organization, and per user."
      />

      {pendingCount > 0 && (
        <div className="sticky top-2 z-10 flex items-center justify-between gap-3 rounded-lg border border-primary/40 bg-primary/10 px-3 py-2 shadow-sm backdrop-blur">
          <span className="text-[12px] font-medium">
            {pendingCount} unsaved change{pendingCount === 1 ? "" : "s"}
          </span>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={() => setDrafts({})}>
              Discard
            </Button>
            <Button variant="primary" size="sm" onClick={applyChanges}>
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

      {isLoading ? (
        <div className="space-y-2" aria-label="Loading feature flags">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-24 w-full" />
          ))}
        </div>
      ) : error ? (
        <FlagsEmptyState
          title="Failed to load feature flags"
          subtitle="Please try again later."
        />
      ) : flags.length === 0 ? (
        <FlagsEmptyState
          title="No feature flags"
          subtitle="Flags are declared in code. None have been defined yet."
        />
      ) : filtered.length === 0 ? (
        <Card>
          <CardContent className="py-6 text-center text-[12px] text-muted-foreground">
            No flags match “{search.trim()}”.
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-2">
          {filtered.map((flag) => (
            <FlagCard
              key={flag.key}
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
              userOptions={userOptions}
              usersLoading={usersLoading}
            />
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
  userOptions,
  usersLoading,
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
  readonly userOptions: readonly { id: string; label: string }[];
  readonly usersLoading: boolean;
}) {
  const [addOrg, setAddOrg] = useState("");
  const [addUser, setAddUser] = useState("");

  const orgIds = uniq([...flag.orgs.map((o) => o.id), ...stagedOrgs]);
  const userIds = uniq([...flag.users.map((u) => u.id), ...stagedUsers]);
  const labelFor = (type: ScopeType, id: string) =>
    type === "user"
      ? userOptions.find((candidate) => candidate.id === id)?.label ?? id
      : id;

  const addableUsers = userOptions.filter((user) => !userIds.includes(user.id));

  // Collapsed summary pills: Global + configured orgs/users (non-inherit).
  const pills: { id: string; label: string; state: ScopeState; pending: boolean }[] =
    [
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
    ].filter((p) => p.id === "global" || p.state !== "inherit");
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
                  title={`${p.label} · ${word(p.state)}`}
                >
                  <span className="min-w-0 truncate">{p.label}</span>
                  <span className="shrink-0">· {word(p.state)}</span>
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

      {open && (
        <div className="space-y-4 border-t border-border/60 bg-muted/20 px-3 py-3">
          <Group label="Global">
            <ScopeRow
              label="All users (rollout / killswitch)"
              state={stateFor("global", "")}
              pending={isPending("global", "")}
              onChange={(s) => stage("global", "", s)}
            />
          </Group>

          <Group label="By organization">
            <AddPicker
              value={addOrg}
              placeholder="Manage overrides from the organization page"
              options={[]}
              onPick={setAddOrg}
              disabledReason="Organization-scoped overrides are managed from each organization page."
            />
            {orgIds.map((id) => (
              <ScopeRow
                key={id}
                label={labelFor("org", id)}
                state={stateFor("org", id)}
                pending={isPending("org", id)}
                onChange={(s) => stage("org", id, s)}
              />
            ))}
          </Group>

          <Group label="By user">
            <AddPicker
              value={addUser}
              placeholder={
                usersLoading
                  ? "Loading users…"
                  : addableUsers.length === 0
                  ? "All users added"
                  : "Add user…"
              }
              options={addableUsers}
              onPick={(id) => {
                setAddUser("");
                stage("user", id, "enabled");
              }}
            />
            {userIds.map((id) => (
              <ScopeRow
                key={id}
                label={labelFor("user", id)}
                state={stateFor("user", id)}
                pending={isPending("user", id)}
                onChange={(s) => stage("user", id, s)}
              />
            ))}
          </Group>
        </div>
      )}
    </Card>
  );
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

function AddPicker({
  value,
  placeholder,
  options,
  onPick,
  disabledReason,
}: {
  readonly value: string;
  readonly placeholder: string;
  readonly options: readonly { id: string; label: string }[];
  readonly onPick: (id: string) => void;
  readonly disabledReason?: string;
}) {
  return (
    <div
      className="border-b border-border/40 px-3 py-2 last:border-b-0"
      title={disabledReason}
    >
      <Select
        value={value}
        onValueChange={onPick}
        disabled={Boolean(disabledReason) || options.length === 0}
      >
        <SelectTrigger className="h-8 w-full text-[12px]">
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.id} value={o.id}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

function ScopeRow({
  label,
  state,
  pending,
  onChange,
}: {
  readonly label: string;
  readonly state: ScopeState;
  readonly pending: boolean;
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
      <Select value={state} onValueChange={(s) => onChange(s as ScopeState)}>
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
  return s === "enabled" ? "Enabled" : s === "disabled" ? "Disabled" : "Inherit";
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
