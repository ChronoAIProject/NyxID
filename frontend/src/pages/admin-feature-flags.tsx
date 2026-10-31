import {
  changedFields,
  describeChanges,
  hasFieldConflicts,
} from "@/lib/form-changes";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  ChevronDown,
  Flag,
  History,
  Search,
  SlidersHorizontal,
  Sunset,
} from "lucide-react";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  DataTableBadgeCell,
  DataTableCellShell,
} from "@/components/data-table/data-table-columns";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn, formatRelativeTime } from "@/lib/utils";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import type { FeatureFlagTarget } from "@/schemas/admin-feature-flags";
import { createFeatureFlagRequestSchema } from "@/schemas/admin-feature-flags";
import { useAppForm } from "@/components/ui/form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Switch } from "@/components/ui/switch";
import {
  useAdminFeatureFlags,
  useCreateFeatureFlag,
  useFeatureFlagTargets,
  useClearAdminFeatureFlag,
  useSetAdminFeatureFlag,
  useUpdateAdminFeatureFlagMetadata,
} from "@/hooks/use-admin-feature-flags";
import {
  MAX_FEATURE_FLAG_DESCRIPTION_LENGTH,
  MAX_FEATURE_FLAG_OWNER_LENGTH,
} from "@/schemas/admin-feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite, canCreateFeatureFlags } from "@/types/api";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { FlagOrganizationEditor } from "@/components/admin/feature-flags/flag-organization-editor";
import {
  FlagBrowserControls,
  type FlagFilters,
} from "@/components/admin/feature-flags/flag-browser-controls";
import { FlagHistory } from "@/components/admin/feature-flags/flag-history";

type ScopeState = "inherit" | "enabled" | "disabled";

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
  readonly group: string | null;
  readonly tags: string[];
  readonly sunset: boolean;
  readonly defaultEnabled: boolean;
  readonly canEdit: boolean;
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
  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canCreateFeatureFlags(currentUser);
  const [drafts, setDrafts] = useState<Record<string, ScopeState>>({});
  const draftVersions = useRef<Record<string, number>>({});
  const draftBaselines = useRef<Record<string, ScopeState>>({});
  const [applying, setApplying] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const createWasOpen = useRef(false);
  useEffect(() => {
    if (createOpen) document.getElementById("new-flag-key")?.focus();
    else if (createWasOpen.current)
      document.getElementById("create-feature-flag")?.focus();
    createWasOpen.current = createOpen;
  }, [createOpen]);
  const [filters, setFilters] = useState<FlagFilters>({
    search: "",
    lifecycle: "active",
    tag: "",
    group: "",
    groupBy: "group",
  });
  const { search, lifecycle, tag, group: groupFilter, groupBy } = filters;
  const [openKeys, setOpenKeys] = useState<string[]>([]);
  const flags: FlagRow[] = (data?.flags ?? []).map((flag) => ({
    key: flag.key,
    description: flag.description,
    codeDescription: flag.code_description ?? flag.description,
    customDescription: flag.custom_description ?? null,
    owner: flag.owner ?? null,
    group: flag.group ?? null,
    tags: flag.tags ?? [],
    sunset: flag.sunset ?? false,
    metadataUpdatedAt: flag.metadata_updated_at ?? null,
    defaultEnabled: flag.default_enabled,
    canEdit: flag.can_edit ?? canAdminWrite(currentUser),
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
  const groupNames = new Map<string, string>();
  for (const flag of flags)
    if (flag.group && !groupNames.has(flag.group.toLowerCase()))
      groupNames.set(flag.group.toLowerCase(), flag.group);
  const groupFor = (flag: FlagRow) => {
    const name =
      flag.group ??
      (flag.key.includes(":") ? flag.key.split(":")[0]! : "General");
    return (
      groupNames.get(name.toLowerCase()) ??
      name[0]!.toUpperCase() + name.slice(1)
    );
  };
  const groups = [...new Set(flags.map(groupFor))].sort((a, b) =>
    a.localeCompare(b),
  );
  const tags = [...new Set(flags.flatMap((flag) => flag.tags))].sort();
  const sunsetCount = flags.filter((flag) => flag.sunset).length;
  const inLifecycle = (flag: FlagRow) =>
    lifecycle === "all" ||
    (lifecycle === "sunset" ? flag.sunset : !flag.sunset);
  const matchesFilters = (flag: FlagRow) =>
    (!tag || flag.tags.includes(tag)) &&
    (!groupFilter ||
      groupFor(flag).toLowerCase() === groupFilter.toLowerCase()) &&
    (!q ||
      [
        flag.key,
        flag.description,
        flag.owner ?? "",
        groupFor(flag),
        ...flag.tags,
      ].some((value) => value.toLowerCase().includes(q)));
  const hiddenSunsetMatches =
    lifecycle === "active"
      ? flags.filter((flag) => flag.sunset && matchesFilters(flag)).length
      : 0;
  const filtered = flags.filter(
    (flag) => inLifecycle(flag) && matchesFilters(flag),
  );
  const sectionFor = (flag: FlagRow) =>
    groupBy === "none"
      ? "All flags"
      : groupBy === "owner"
        ? (flag.owner ?? "Unassigned")
        : groupFor(flag);
  const ordered = [...flags].sort(
    (a, b) =>
      sectionFor(a).localeCompare(sectionFor(b)) || a.key.localeCompare(b.key),
  );
  const visibleKeys = new Set(filtered.map((flag) => flag.key));
  const firstInSection = new Map<string, string>();
  for (const flag of ordered)
    if (visibleKeys.has(flag.key) && !firstInSection.has(sectionFor(flag)))
      firstInSection.set(sectionFor(flag), flag.key);
  const sectionCounts = new Map<string, number>();
  for (const flag of filtered)
    sectionCounts.set(
      sectionFor(flag),
      (sectionCounts.get(sectionFor(flag)) ?? 0) + 1,
    );

  const isOpen = (key: string) => (q ? true : openKeys.includes(key));
  const toggle = (key: string) =>
    setOpenKeys((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key],
    );
  const pendingKeys = new Set(pending.map(([k]) => k));
  const dirtyFlags = new Set(pending.map(([k]) => k.split("\u001f")[0] ?? ""));

  const hasFilters = Boolean(q || tag || groupFilter);
  const lifecycleCounts = {
    active: flags.length - sunsetCount,
    sunset: sunsetCount,
    all: flags.length,
  } as const;

  return (
    <div className="space-y-6">
      {rolloutReview.dialog}
      <PageHeader
        title="Feature Flags"
        description="Organize rollouts, track every change, and keep finished work out of the way."
        actions={
          canWrite && (
            <AddCtaButton
              id="create-feature-flag"
              label="Create feature flag"
              variant="subtle"
              aria-expanded={createOpen}
              aria-controls="create-feature-flag-form"
              disabled={createOpen}
              onClick={() => setCreateOpen(true)}
            />
          )
        }
      />

      {canWrite && (
        <CreateFlagForm
          open={createOpen}
          onClose={() => setCreateOpen(false)}
        />
      )}

      {canWrite && pendingCount > 0 && (
        <div className="sticky top-2 z-20 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-warning/30 bg-card px-3 py-2 shadow-sm">
          <span className="flex items-center gap-2 text-12 font-medium text-foreground">
            <span
              aria-hidden
              className="size-1.5 shrink-0 rounded-full bg-warning"
            />
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

      <Tabs
        value={lifecycle}
        onValueChange={(value) =>
          setFilters((current) => ({
            ...current,
            lifecycle: value as FlagFilters["lifecycle"],
          }))
        }
      >
        <TabsList aria-label="Flag lifecycle">
          {(["active", "sunset", "all"] as const).map((view) => (
            <TabsTrigger key={view} value={view} className="gap-1.5">
              {view === "active"
                ? "Active"
                : view === "sunset"
                  ? "Sunset"
                  : "All flags"}
              <span className="text-11 tabular-nums text-text-tertiary">
                {lifecycleCounts[view]}
              </span>
            </TabsTrigger>
          ))}
        </TabsList>
        {/* One panel serves every lifecycle view: the table stays mounted so
            hidden rows keep their drafts when the view changes. */}
        <TabsContent value={lifecycle} forceMount className="mt-4">
          <section
            aria-label="Feature flags"
            className="overflow-hidden rounded-lg border border-border/60 bg-card"
          >
            <FlagBrowserControls
              filters={filters}
              onChange={(patch) =>
                setFilters((current) => ({ ...current, ...patch }))
              }
              groups={groups}
              tags={tags}
              totalCount={flags.length}
              sunsetCount={sunsetCount}
              visibleCount={filtered.length}
            />
            {error && data && (
              <p
                role="alert"
                className="flex items-center gap-2 border-b border-destructive/25 bg-destructive/5 px-3 py-2.5 text-12 text-destructive"
              >
                <AlertTriangle className="size-4 shrink-0" aria-hidden />
                Unable to refresh feature flags. Your drafts are retained.
              </p>
            )}
            {lifecycle === "sunset" && (
              <p className="flex items-start gap-2 border-b border-warning/30 bg-warning/5 px-3 py-2.5 text-12 text-muted-foreground">
                <Sunset
                  className="mt-0.5 size-4 shrink-0 text-warning"
                  aria-hidden
                />
                Sunset flags are hidden from Active. Their defaults and
                overrides still apply until the corresponding code is removed.
              </p>
            )}
            {hiddenSunsetMatches > 0 && hasFilters && (
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-warning/30 bg-warning/5 px-3 py-1.5 text-12 text-muted-foreground">
                <span className="flex items-center gap-2">
                  <Sunset
                    className="size-4 shrink-0 text-warning"
                    aria-hidden
                  />
                  {hiddenSunsetMatches} matching flag
                  {hiddenSunsetMatches === 1 ? " is" : "s are"} marked for
                  sunset.
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    setFilters((current) => ({
                      ...current,
                      lifecycle: "sunset",
                    }))
                  }
                >
                  View sunset matches
                </Button>
              </div>
            )}

            {isLoading && !data ? (
              <div
                className="space-y-2 p-3"
                role="status"
                aria-label="Loading feature flags"
              >
                {Array.from({ length: 4 }).map((_, index) => (
                  <Skeleton key={index} className="h-12 w-full" />
                ))}
              </div>
            ) : error && !data ? (
              <FlagsEmptyState
                tone="error"
                title="Failed to load feature flags"
                subtitle="Please try again later."
              />
            ) : flags.length === 0 ? (
              <FlagsEmptyState
                title="No feature flags"
                subtitle="No feature flags are available to your account. Create a flag to get started."
              />
            ) : (
              <>
                {filtered.length === 0 && (
                  <FlagsEmptyState
                    title={
                      hasFilters
                        ? "No matching flags"
                        : lifecycle === "sunset"
                          ? "No flags marked for sunset"
                          : "No active flags"
                    }
                    subtitle={
                      hasFilters
                        ? "Try another search or clear your filters."
                        : lifecycle === "sunset"
                          ? "Mark a flag for sunset in its organization settings when code cleanup is pending."
                          : "Use All flags to see flags marked for sunset, or create a new flag."
                    }
                  />
                )}
                {/* Every flag stays mounted (filtered rows are hidden, not
                    removed) so open editors keep their drafts. */}
                <Table
                  containerClassName={cn(filtered.length === 0 && "hidden")}
                >
                  <TableHeader>
                    <TableRow>
                      <TableHead>Flag</TableHead>
                      <TableHead className="w-[42%] md:w-56">Rollout</TableHead>
                      <TableHead className="hidden w-24 md:table-cell">
                        Default
                      </TableHead>
                      <TableHead className="hidden w-36 lg:table-cell">
                        Group
                      </TableHead>
                      <TableHead className="hidden w-40 lg:table-cell">
                        Owner
                      </TableHead>
                      <TableHead className="hidden w-48 xl:table-cell">
                        Tags
                      </TableHead>
                      <TableHead className="w-10">
                        <span className="sr-only">Details</span>
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  {ordered.map((flag) => (
                    <FlagTableRows
                      key={flag.key}
                      flag={flag}
                      hidden={!visibleKeys.has(flag.key)}
                      section={
                        groupBy !== "none" &&
                        firstInSection.get(sectionFor(flag)) === flag.key
                          ? {
                              label: sectionFor(flag),
                              count: sectionCounts.get(sectionFor(flag)) ?? 0,
                            }
                          : null
                      }
                      groupLabel={groupFor(flag)}
                      groups={groups}
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
                      canWrite={flag.canEdit}
                    />
                  ))}
                </Table>
              </>
            )}
          </section>
        </TabsContent>
      </Tabs>
    </div>
  );
}

/** Columns in the flag table, so section and editor rows span all of them. */
const FLAG_TABLE_COLUMNS = 7;

function CreateFlagForm({
  open,
  onClose,
}: {
  readonly open: boolean;
  readonly onClose: () => void;
}) {
  const create = useCreateFeatureFlag();
  const form = useAppForm({
    resolver: zodResolver(createFeatureFlagRequestSchema),
    defaultValues: { key: "", description: "", default_enabled: false },
  });
  if (!open) return null;
  return (
    <Card className="space-y-3 p-4">
      <form
        id="create-feature-flag-form"
        className="space-y-3"
        onSubmit={form.handleSubmit(async (body) => {
          try {
            await create.mutateAsync(body);
            toast.success("Feature flag created");
            form.reset();
            onClose();
          } catch (error) {
            toast.error(
              error instanceof Error
                ? error.message
                : "Failed to create feature flag",
            );
          }
        })}
      >
        <div>
          <Label htmlFor="new-flag-key">Flag key</Label>
          <Input
            id="new-flag-key"
            {...form.register("key")}
            placeholder="my-team:new-feature"
            disabled={create.isPending}
          />
          {form.formState.errors.key && (
            <p role="alert" className="text-12 text-destructive">
              {form.formState.errors.key.message}
            </p>
          )}
        </div>
        <div>
          <Label htmlFor="new-flag-description">Description</Label>
          <Input
            id="new-flag-description"
            {...form.register("description")}
            maxLength={MAX_FEATURE_FLAG_DESCRIPTION_LENGTH}
            disabled={create.isPending}
          />
          {form.formState.errors.description && (
            <p role="alert" className="text-12 text-destructive">
              {form.formState.errors.description.message}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Switch
            id="new-flag-default"
            checked={form.watch("default_enabled")}
            onCheckedChange={(checked) =>
              form.setValue("default_enabled", checked)
            }
            disabled={create.isPending}
          />
          <Label htmlFor="new-flag-default">
            Enabled by default for everyone
          </Label>
        </div>
        <div className="flex gap-2">
          <Button
            type="submit"
            variant="primary"
            isLoading={create.isPending}
            disabled={
              create.isPending ||
              !form.formState.isDirty ||
              !form.watch("key").trim() ||
              !form.watch("description").trim()
            }
          >
            Create flag
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={create.isPending}
            onClick={onClose}
          >
            Close
          </Button>
        </div>
      </form>
    </Card>
  );
}

function FlagTableRows({
  flag,
  hidden,
  section,
  groupLabel,
  open,
  dirty,
  onToggle,
  stateFor,
  isPending,
  stage,
  stagedOrgs,
  stagedUsers,
  canWrite,
  groups,
}: {
  readonly flag: FlagRow;
  readonly hidden: boolean;
  /** Set on the first visible flag of a section when the table is grouped. */
  readonly section: { readonly label: string; readonly count: number } | null;
  readonly groupLabel: string;
  readonly groups: string[];
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
  const [panel, setPanel] = useState<"configuration" | "history">(
    "configuration",
  );
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

  // Rollout summary of the stored (or staged) overrides. It describes
  // configuration, not any one person's evaluated result.
  const globalState = stateFor("global", "");
  const globallyOn =
    globalState === "inherit" ? flag.defaultEnabled : globalState === "enabled";
  const targeted = [
    ...orgIds.map((id) => ({
      kind: "org" as const,
      label: labelFor("org", id),
      state: stateFor("org", id),
    })),
    ...userIds.map((id) => ({
      kind: "user" as const,
      label: labelFor("user", id),
      state: stateFor("user", id),
    })),
  ].filter((entry) => entry.state !== "inherit");
  const targetedOrgs = targeted.filter((entry) => entry.kind === "org").length;
  const targetedUsers = targeted.length - targetedOrgs;
  const shownTags = flag.tags.slice(0, 3);
  const editorId = `flag-editor-${flag.key}`;

  return (
    <tbody hidden={hidden} data-testid={`flag-${flag.key}`}>
      {section && (
        <TableRow className="bg-muted/30 hover:bg-muted/30">
          <TableCell
            colSpan={FLAG_TABLE_COLUMNS}
            className="py-1.5 text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary"
          >
            {section.label}
            <span className="ml-2 font-normal tabular-nums">
              {section.count}
            </span>
          </TableCell>
        </TableRow>
      )}
      <TableRow
        className={cn("cursor-pointer", open && "bg-overlay")}
        onClick={(event) => {
          // The key is the accessible toggle; the rest of the row is a larger
          // mouse target, as on the other admin lists.
          if (
            (event.target as HTMLElement).closest(
              "button, a, input, select, textarea",
            )
          )
            return;
          onToggle();
        }}
      >
        <TableCell>
          <DataTableCellShell>
            <div className="min-w-0 space-y-0.5">
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={onToggle}
                  aria-expanded={open}
                  aria-controls={editorId}
                  className="rounded-sm text-left font-mono text-13 font-semibold text-foreground [overflow-wrap:anywhere] underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                >
                  <BreakableKey value={flag.key} />
                </button>
                {flag.sunset && <Badge variant="warning">Sunset</Badge>}
              </div>
              <p
                className="line-clamp-1 break-words text-12 text-muted-foreground"
                title={flag.description}
              >
                {flag.description}
              </p>
            </div>
          </DataTableCellShell>
        </TableCell>
        <TableCell>
          <DataTableCellShell>
            <DataTableBadgeCell>
              <Badge
                variant={globallyOn ? "success" : "secondary"}
                title={`Global · ${word(globalState)}`}
              >
                {globalState === "enabled"
                  ? "On globally"
                  : globalState === "disabled"
                    ? "Off globally"
                    : `Default · ${flag.defaultEnabled ? "on" : "off"}`}
              </Badge>
              {targeted.length > 0 && (
                <Badge
                  variant="info"
                  title={targeted
                    .map((entry) => `${entry.label}: ${word(entry.state)}`)
                    .join("\n")}
                >
                  {[
                    targetedOrgs > 0 &&
                      `${targetedOrgs} org${targetedOrgs === 1 ? "" : "s"}`,
                    targetedUsers > 0 &&
                      `${targetedUsers} user${targetedUsers === 1 ? "" : "s"}`,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </Badge>
              )}
              {dirty && <Badge variant="warning">Unsaved</Badge>}
            </DataTableBadgeCell>
          </DataTableCellShell>
        </TableCell>
        <TableCell className="hidden text-muted-foreground md:table-cell">
          {flag.defaultEnabled ? "Enabled" : "Disabled"}
        </TableCell>
        <TableCell className="hidden lg:table-cell">
          <span className="line-clamp-2 break-words" title={groupLabel}>
            {groupLabel}
          </span>
        </TableCell>
        <TableCell className="hidden lg:table-cell">
          {flag.owner ? (
            <span className="line-clamp-2 break-words" title={flag.owner}>
              {flag.owner}
            </span>
          ) : (
            <span className="text-text-tertiary">--</span>
          )}
        </TableCell>
        <TableCell className="hidden xl:table-cell">
          {flag.tags.length > 0 ? (
            <DataTableBadgeCell>
              {shownTags.map((tag) => (
                <Badge key={tag} variant="secondary" className="break-all">
                  {tag}
                </Badge>
              ))}
              {flag.tags.length > shownTags.length && (
                <span
                  className="text-11 text-text-tertiary"
                  title={flag.tags.join(", ")}
                >
                  +{flag.tags.length - shownTags.length}
                </span>
              )}
            </DataTableBadgeCell>
          ) : (
            <span className="text-text-tertiary">--</span>
          )}
        </TableCell>
        <TableCell className="text-right">
          <ChevronDown
            aria-hidden
            className={cn(
              "ml-auto h-4 w-4 text-muted-foreground transition-transform",
              open && "rotate-180",
            )}
          />
        </TableCell>
      </TableRow>
      <TableRow
        id={editorId}
        hidden={!open}
        className="bg-muted/20 hover:bg-muted/20"
      >
        <TableCell colSpan={FLAG_TABLE_COLUMNS} className="p-0">
          <Tabs
            value={panel}
            onValueChange={(value) =>
              setPanel(value as "configuration" | "history")
            }
            className="px-4 pb-5 pt-2 sm:px-6"
          >
            <TabsList aria-label={`${flag.key} details`}>
              <TabsTrigger value="configuration" className="gap-1.5">
                <SlidersHorizontal className="size-3.5" aria-hidden />
                Configuration
              </TabsTrigger>
              <TabsTrigger value="history" className="gap-1.5">
                <History className="size-3.5" aria-hidden />
                History
              </TabsTrigger>
            </TabsList>
            {/* Both panels stay mounted so editor drafts survive switching. */}
            <TabsContent
              value="history"
              forceMount
              className="mt-4 data-[state=inactive]:hidden"
            >
              <FlagHistory
                flagKey={flag.key}
                enabled={open && panel === "history"}
              />
            </TabsContent>
            <TabsContent
              value="configuration"
              forceMount
              className="mt-4 space-y-4 data-[state=inactive]:hidden"
            >
              <Group label="Organization">
                <FlagOrganizationEditor
                  flagKey={flag.key}
                  current={{
                    group: flag.group,
                    tags: flag.tags,
                    sunset: flag.sunset,
                  }}
                  canWrite={canWrite}
                  groups={groups}
                />
              </Group>
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

              {flag.key !== FEATURE_FLAG.INVITATION_CODE && (
                <Group label="By organization">
                  {canWrite && (
                    <AccountSearchPicker
                      flagKey={flag.key}
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
              )}

              {flag.key !== FEATURE_FLAG.INVITATION_CODE && (
                <Group label="By user">
                  {canWrite && (
                    <AccountSearchPicker
                      flagKey={flag.key}
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
                    const assignment = flag.users.find(
                      (user) => user.id === id,
                    );
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
              )}
            </TabsContent>
          </Tabs>
        </TableCell>
      </TableRow>
    </tbody>
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
      <dl className="space-y-2 text-12">
        <div className="space-y-0.5">
          <dt className="text-11 text-muted-foreground">Description</dt>
          <dd className="text-foreground">{flag.description}</dd>
        </div>
        <div className="space-y-0.5">
          <dt className="text-11 text-muted-foreground">Owner</dt>
          <dd className={cn(!flag.owner && "text-muted-foreground")}>
            {flag.owner ?? "Unassigned"}
          </dd>
        </div>
      </dl>
    );
  }

  return (
    <div className="space-y-2.5">
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
          className="text-11 font-normal"
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
          className="h-8 text-12"
        />
      </div>
      <div className="space-y-1">
        <Label
          htmlFor={`flag-owner-${flag.key}`}
          className="text-11 font-normal"
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
          className="h-8 text-12"
        />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-11 text-muted-foreground">
          {flag.customDescription == null
            ? "Using the original description."
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
              Use default description
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

/** Default suggestions shown on focus before the admin types anything. */
const DEFAULT_SUGGESTION_COUNT = 5;
/** Over-fetch so exclusions don't leave the suggestion list short. */
const DEFAULT_SUGGESTION_FETCH = 8;

function AccountSearchPicker({
  flagKey,
  kind,
  excludedIds,
  onPick,
}: {
  readonly flagKey: string;
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

  const { data, isLoading } = useFeatureFlagTargets(
    flagKey,
    kind,
    debouncedSearch || undefined,
    20,
    focused && !!debouncedSearch,
  );
  const waitingForSearch = normalizedSearch !== debouncedSearch;
  const results =
    debouncedSearch && !waitingForSearch
      ? (data?.users ?? []).filter(
          (account) => !excludedIds.includes(account.id),
        )
      : [];

  // A flag card renders one picker per scope, so only fetch defaults while this
  // picker's dropdown is actually open; repeated queries for this flag dedupe.
  const showDefaults = focused && !normalizedSearch;
  const { data: defaultsData, isLoading: defaultsLoading } =
    useFeatureFlagTargets(
      flagKey,
      kind,
      undefined,
      DEFAULT_SUGGESTION_FETCH,
      showDefaults,
    );
  const defaults = (defaultsData?.users ?? [])
    .filter((account) => !excludedIds.includes(account.id))
    .slice(0, DEFAULT_SUGGESTION_COUNT);
  const hasMoreSuggestions =
    (defaultsData?.users.length ?? 0) > defaults.length;

  const isOrg = kind === "org";
  const pick = (account: FeatureFlagTarget) => {
    onPick({ id: account.id, label: accountLabel(account, isOrg) });
    setSearch("");
    setDebouncedSearch("");
  };

  return (
    <div
      className="space-y-2 border-b border-border/40 py-2 last:border-b-0"
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
          className="h-8 pl-8 text-12"
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
                {hasMoreSuggestions && (
                  <p className="border-t border-border/40 px-3 py-2 text-11 text-muted-foreground">
                    More accounts — keep typing to narrow
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
  readonly account: FeatureFlagTarget;
  readonly isOrg: boolean;
  readonly onPick: (account: FeatureFlagTarget) => void;
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

function accountLabel(account: FeatureFlagTarget, isOrg: boolean): string {
  return isOrg
    ? (account.display_name ?? account.slug ?? account.id)
    : account.email;
}

/** In-card empty and error states, matching the audit log's. */
function FlagsEmptyState({
  title,
  subtitle,
  tone = "neutral",
}: {
  readonly title: string;
  readonly subtitle: string;
  readonly tone?: "neutral" | "error";
}) {
  const error = tone === "error";
  return (
    <div
      role={error ? "alert" : undefined}
      className={cn(
        "flex min-h-56 flex-col items-center justify-center gap-3 px-4 py-14 text-center",
        error && "bg-destructive/5",
      )}
    >
      {error ? (
        <AlertTriangle className="h-8 w-8 text-destructive" aria-hidden />
      ) : (
        <Flag className="h-10 w-10 text-muted-foreground" aria-hidden />
      )}
      <div>
        <p
          className={cn(
            "text-sm font-medium",
            error ? "text-destructive" : "text-foreground",
          )}
        >
          {title}
        </p>
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
    <div className="grid gap-2 border-t border-border/40 pt-4 first:border-t-0 first:pt-0 sm:grid-cols-[8rem_minmax(0,1fr)] sm:gap-4">
      <p className="text-11 font-semibold text-text-tertiary sm:pt-1.5">
        {label}
      </p>
      <div className="min-w-0">{children}</div>
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
        "flex items-center justify-between gap-3 py-1.5",
        pending && "bg-primary/[0.07]",
      )}
    >
      <span className="flex min-w-0 items-center gap-1.5 text-12 font-medium text-foreground">
        {/* Dot slot is always reserved so labels don't shift when a row
            becomes pending; only its color/visibility changes. */}
        <span
          aria-hidden
          className={cn(
            "h-1.5 w-1.5 shrink-0 rounded-full bg-primary",
            !pending && "invisible",
          )}
        />
        <span className="min-w-0 [overflow-wrap:anywhere]">{label}</span>
      </span>
      <Select
        value={state}
        disabled={disabled}
        onValueChange={(s) => onChange(s as ScopeState)}
      >
        <SelectTrigger className="h-7 w-30 shrink-0 text-12" aria-label={label}>
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

/**
 * A flag key that wraps after `:`, `-` or `_` before breaking mid-word. The
 * parts stay direct text nodes of the caller's element, so the key still reads
 * (and is found) as one string.
 */
function BreakableKey({ value }: { readonly value: string }) {
  return value
    .split(/(?<=[:_-])/)
    .flatMap((part, index) =>
      index === 0 ? [part] : [<wbr key={`break-${index}`} />, part],
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
