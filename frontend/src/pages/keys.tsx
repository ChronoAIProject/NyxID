import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearch, useNavigate } from "@tanstack/react-router";
import { useKeys } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import {
  Globe,
  KeyRound,
  KeySquare,
  Server,
  Router,
  Terminal,
  Zap,
} from "lucide-react";
import { MagicKeyIcon } from "@/components/icons/empty-state";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { useNodes } from "@/hooks/use-nodes";
import { ViewToggle, useViewMode, type ViewMode } from "@/components/shared/view-toggle";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";
import { ApiKeyUsageDashboard } from "@/components/dashboard/api-key-usage-dashboard";
import { RoleBadge } from "@/components/orgs/role-badge";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import type { KeyInfo } from "@/types/keys";
import type { CredentialSource } from "@/schemas/orgs";
import {
  KEYS_TABS,
  KEYS_TAB_DEFAULT,
  KEYS_ACTIONS,
  type KeysAction,
  type KeysTab,
  isValidTab,
  parseTab,
} from "@/lib/url-tabs";

function statusVariant(
  status: string,
): "success" | "secondary" | "destructive" {
  switch (status) {
    case "active":
      return "success";
    case "expired":
      return "secondary";
    case "revoked":
    case "failed":
    case "refresh_failed":
      return "destructive";
    default:
      return "secondary";
  }
}

interface KeyCardProps {
  readonly keyInfo: KeyInfo;
  /** Credential provenance; undefined is treated as personal. */
  readonly source: CredentialSource | undefined;
}

function KeyCardContent({ keyInfo, source }: KeyCardProps) {
  const isSsh = keyInfo.service_type === "ssh";
  const hasSshCertificateAuth = isSsh && keyInfo.ssh_ca_public_key !== null;
  // Issue #416: resolve the bound node's name so the list card shows
  // "Via my-node" instead of bare "Via node". TanStack Query dedupes
  // the request across all rendered cards.
  const { data: nodes } = useNodes();
  const nodeName = keyInfo.node_id
    ? (nodes?.find((n) => n.id === keyInfo.node_id)?.name ??
      keyInfo.node_id.slice(0, 8))
    : null;
  const displayUrl = isSsh
    ? `${keyInfo.ssh_host ?? "unknown"}:${keyInfo.ssh_port ?? 22}`
    : keyInfo.endpoint_url.length > 50
      ? `${keyInfo.endpoint_url.slice(0, 50)}...`
      : keyInfo.endpoint_url;

  const isOrgInherited = source?.type === "org";
  // Viewers and out-of-scope members see the card with reduced opacity.
  const isBlocked = source?.type === "org" && !source.allowed;
  // Members can USE the credential (allowed=true) but cannot MODIFY it.
  // Non-admin org cards are non-clickable on the listing (see KeyCard)
  // and flagged as read-only so the user knows why.
  const isReadOnly =
    source?.type === "org" && source.allowed && source.role !== "admin";

  return (
    <Card
      className={`h-full transition-colors duration-300 ${
        isBlocked
          ? "opacity-60"
          : "hover:border-white/[0.15] hover:bg-accent/30"
      }`}
      aria-disabled={isBlocked ? true : undefined}
    >
      <CardContent className="flex h-full min-h-[140px] flex-col gap-3 p-4">
        <div className="min-w-0">
          <p className="truncate text-[12px] font-medium text-foreground">
            {keyInfo.label}
          </p>
          {keyInfo.catalog_service_name && (
            <p className="truncate text-xs text-muted-foreground">
              {keyInfo.catalog_service_name}
            </p>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {isOrgInherited && (
            <Badge variant="info">Org</Badge>
          )}
          {isBlocked && (
            <Badge variant="secondary">Read-Only</Badge>
          )}
          {isReadOnly && !isBlocked && (
            <Badge variant="secondary">View-Only</Badge>
          )}
          <Badge variant={statusVariant(keyInfo.status)}>
            {keyInfo.status.charAt(0).toUpperCase() + keyInfo.status.slice(1)}
          </Badge>
          {isSsh && <Badge variant="secondary">SSH</Badge>}
          {keyInfo.auto_connected && (
            <Badge variant="secondary">
              {keyInfo.source_app_name
                ? `Via ${keyInfo.source_app_name}`
                : "Auto-connected"}
            </Badge>
          )}
          {!keyInfo.is_active && <Badge variant="secondary">Inactive</Badge>}
        </div>

        <div className="mt-auto grid grid-cols-2 gap-x-4 gap-y-3 text-xs text-muted-foreground">
          <div className="flex min-w-0 items-center gap-1.5">
            {isSsh ? (
              <Terminal className="h-3 w-3 shrink-0" />
            ) : (
              <Globe className="h-3 w-3 shrink-0" />
            )}
            <span className="truncate">{displayUrl}</span>
          </div>
          <div className="flex min-w-0 items-center justify-end gap-1.5">
            {keyInfo.auto_connected ? (
              <Zap className="h-3 w-3 shrink-0" />
            ) : (
              <KeyRound className="h-3 w-3 shrink-0" />
            )}
            <span className="truncate">
              {keyInfo.auto_connected
                ? "No auth required"
                : isSsh
                  ? hasSshCertificateAuth
                    ? "certificate"
                    : "ssh tunnel"
                  : keyInfo.credential_type}
            </span>
          </div>
          <div className="flex min-w-0 items-center gap-1.5">
            <Server className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {isSsh ? keyInfo.slug : `/proxy/s/${keyInfo.slug}`}
            </span>
          </div>
          <div className="flex min-w-0 items-center justify-end gap-1.5">
            <Router className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {nodeName ? `→ ${nodeName}` : "Direct"}
            </span>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function KeyCard({ keyInfo, source }: KeyCardProps) {
  // Navigation gating:
  //
  // - Personal credentials and admin-role org credentials: fully clickable
  //   and the detail page renders all mutation controls.
  // - Member / viewer org credentials: clickable, but the detail page
  //   renders in read-only mode (see KeyDetailPage's `readOnly` branch).
  //   Members can still see endpoint / auth metadata and a usage snippet
  //   for credentials they're entitled to proxy through.
  // - Out-of-scope org items (source.allowed === false due to scope, not
  //   role) don't even appear in the listing because
  //   `list_user_services_with_sources` drops them.
  return (
    <Link to="/keys/$keyId" params={{ keyId: keyInfo.id }} className="h-full">
      <KeyCardContent keyInfo={keyInfo} source={source} />
    </Link>
  );
}

function ServiceTableRow({
  keyInfo,
  source,
}: KeyCardProps) {
  const navigate = useNavigate();
  const isSsh = keyInfo.service_type === "ssh";
  const hasSshCertificateAuth = isSsh && keyInfo.ssh_ca_public_key !== null;
  const { data: nodes } = useNodes();
  const nodeName = keyInfo.node_id
    ? (nodes?.find((n) => n.id === keyInfo.node_id)?.name ??
      keyInfo.node_id.slice(0, 8))
    : null;

  const isOrgInherited = source?.type === "org";
  const isBlocked = source?.type === "org" && !source.allowed;
  const isReadOnly =
    source?.type === "org" && source.allowed && source.role !== "admin";

  const displayUrl = isSsh
    ? `${keyInfo.ssh_host ?? "unknown"}:${keyInfo.ssh_port ?? 22}`
    : keyInfo.endpoint_url;

  const authLabel = keyInfo.auto_connected
    ? "No auth"
    : isSsh
      ? hasSshCertificateAuth
        ? "certificate"
        : "ssh tunnel"
      : keyInfo.credential_type;

  return (
    <TableRow
      className={`border-border/30 cursor-pointer hover:bg-white/[0.03] ${isBlocked ? "opacity-60" : ""}`}
      onClick={() => void navigate({ to: "/keys/$keyId", params: { keyId: keyInfo.id } })}
    >
      <TableCell className="h-[60px]">
        <p className="truncate font-medium text-foreground">{keyInfo.label}</p>
        <p className="truncate text-[11px] text-text-tertiary mt-0.5">
          {keyInfo.catalog_service_name ?? " "}
        </p>
      </TableCell>

      <TableCell className="h-[60px]">
        <span className="truncate text-muted-foreground text-[11px] font-mono">
          {displayUrl}
        </span>
      </TableCell>

      <TableCell className="h-[60px] text-muted-foreground">{authLabel}</TableCell>

      <TableCell className="h-[60px]">
        <span className="truncate text-muted-foreground text-[11px] font-mono">
          {isSsh ? keyInfo.slug : `/proxy/s/${keyInfo.slug}`}
        </span>
      </TableCell>

      <TableCell className="h-[60px] text-muted-foreground">
        {nodeName ? `→ ${nodeName}` : "Direct"}
      </TableCell>

      <TableCell className="h-[60px]">
        <div className="flex flex-wrap gap-1">
          {isOrgInherited && <Badge variant="info">Org</Badge>}
          {isBlocked && <Badge variant="secondary">Read-Only</Badge>}
          {isReadOnly && !isBlocked && <Badge variant="secondary">View-Only</Badge>}
          <Badge variant={statusVariant(keyInfo.status)}>
            {keyInfo.status.charAt(0).toUpperCase() + keyInfo.status.slice(1)}
          </Badge>
          {isSsh && <Badge variant="secondary">SSH</Badge>}
        </div>
      </TableCell>
    </TableRow>
  );
}

function ServiceTableView({
  groups,
}: {
  readonly groups: readonly ServiceGroup[];
}) {
  return (
    <div className="space-y-8">
      {groups.map((group) => (
        <section key={group.key} className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2">
              {group.icon === "org" ? (
                <OrgAvatar
                  avatarUrl={group.avatarUrl}
                  displayName={group.title}
                  className="h-6 w-6 text-[0.625rem]"
                />
              ) : (
                <Globe className="h-4 w-4 text-muted-foreground" />
              )}
              <h3 className="text-[13px] font-semibold text-foreground">
                {group.title}
              </h3>
            </div>
            {group.role && <RoleBadge role={group.role} />}
            {group.subtitle && (
              <span className="text-xs text-muted-foreground">
                {group.subtitle}
              </span>
            )}
          </div>
          <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow className="border-border/50 hover:bg-transparent">
                  <TableHead className="w-[20%]">Name</TableHead>
                  <TableHead className="w-[22%]">Endpoint</TableHead>
                  <TableHead className="w-[10%]">Auth</TableHead>
                  <TableHead className="w-[20%]">Proxy Slug</TableHead>
                  <TableHead className="w-[10%]">Routing</TableHead>
                  <TableHead className="w-[18%]">Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {group.keys.map(({ keyInfo, source }) => (
                  <ServiceTableRow key={keyInfo.id} keyInfo={keyInfo} source={source} />
                ))}
              </TableBody>
            </Table>
          </div>
        </section>
      ))}
    </div>
  );
}

interface ServiceGroup {
  readonly key: string;
  readonly title: string;
  readonly subtitle: string | null;
  readonly role: "owner" | "admin" | "member" | "viewer" | null;
  readonly icon: "personal" | "org";
  /**
   * Org avatar URL when `icon === "org"`. Surfaced via `credential_source`
   * on the API response so we can render the same avatar as the
   * Organizations page (#545). `null` when the org has no avatar configured
   * — falls back to initials / building icon inside `OrgAvatar`.
   */
  readonly avatarUrl: string | null;
  readonly keys: readonly {
    readonly keyInfo: KeyInfo;
    readonly source: CredentialSource;
  }[];
}

/**
 * Group visible keys by credential source. Personal items first, then one
 * section per org (ordered by first-seen in the incoming list).
 *
 * Keys without an explicit `credential_source` default to `personal` so the
 * UI keeps working against older backends that have not yet been augmented.
 */
function groupKeysBySource(
  keys: readonly KeyInfo[],
  sourceById: ReadonlyMap<string, CredentialSource>,
): readonly ServiceGroup[] {
  const personal: ServiceGroup = {
    key: "personal",
    title: "My Services",
    subtitle: null,
    role: null,
    icon: "personal",
    avatarUrl: null,
    keys: [],
  };

  const orgGroups = new Map<string, ServiceGroup>();
  const personalMut: { keyInfo: KeyInfo; source: CredentialSource }[] = [];

  for (const keyInfo of keys) {
    const source: CredentialSource = keyInfo.credential_source ??
      sourceById.get(keyInfo.id) ?? { type: "personal" };

    if (source.type === "personal") {
      personalMut.push({ keyInfo, source });
      continue;
    }

    const existing = orgGroups.get(source.org_id);
    if (existing) {
      orgGroups.set(source.org_id, {
        ...existing,
        // Prefer the first non-null avatar we see for this org. The backend
        // returns the same avatar on every row, but when `/keys` loads
        // before /user-services has finished hydrating the source map, the
        // earliest entry may lack it — keep whichever value we've already
        // captured.
        avatarUrl: existing.avatarUrl ?? source.avatar_url ?? null,
        keys: [...existing.keys, { keyInfo, source }],
      });
    } else {
      orgGroups.set(source.org_id, {
        key: `org-${source.org_id}`,
        title: source.org_name,
        subtitle: "Shared from organization",
        role: source.role,
        icon: "org",
        avatarUrl: source.avatar_url ?? null,
        keys: [{ keyInfo, source }],
      });
    }
  }

  const groups: ServiceGroup[] = [];
  if (personalMut.length > 0) {
    groups.push({ ...personal, keys: personalMut });
  }
  for (const g of orgGroups.values()) {
    groups.push(g);
  }
  return groups;
}

function ServicesEmptyState({ onAdd }: { readonly onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
      <MagicKeyIcon className="h-64 w-64 text-muted-foreground/30" />
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-muted-foreground/30">No AI services yet</p>
        <p className="text-xs text-muted-foreground/30">
          Connect a downstream service (OpenAI, GitHub, Anthropic, etc.) so your
          AI agents can call it through NyxID without ever seeing the raw key.
        </p>
      </div>
      <AddCtaButton label="Add Service" onClick={onAdd} />
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }, (_, i) => (
        <Skeleton key={i} className="h-32 rounded-xl" />
      ))}
    </div>
  );
}

function ExternalServicesTab({
  onAdd,
  showAutoConnected,
  viewMode,
}: {
  readonly onAdd: () => void;
  readonly showAutoConnected: boolean;
  readonly viewMode: ViewMode;
}) {
  const { data: keys, isLoading, error, refetch } = useKeys();
  // user-services carries credential_source for both personal and
  // org-inherited items. When the backend augments /keys directly in a
  // future change, the `credential_source` field on KeyInfo will take
  // precedence and this call becomes a no-op.
  const { data: userServices } = useUserServices();

  const sourceById = useMemo(() => {
    const map = new Map<string, CredentialSource>();
    for (const svc of userServices ?? []) {
      map.set(svc.id, svc.credential_source);
    }
    return map;
  }, [userServices]);

  if (isLoading) return <LoadingSkeleton />;

  if (error) {
    return (
      <ErrorBanner message="Failed to load services. Please try again." onRetry={refetch} />
    );
  }

  const userKeys = (keys ?? []).filter((k) => !k.auto_connected);
  const autoKeys = (keys ?? []).filter((k) => k.auto_connected);
  const visibleKeys = showAutoConnected ? (keys ?? []) : userKeys;

  if (visibleKeys.length === 0 && autoKeys.length === 0) {
    return <ServicesEmptyState onAdd={onAdd} />;
  }

  if (visibleKeys.length === 0) {
    return <ServicesEmptyState onAdd={onAdd} />;
  }

  const groups = groupKeysBySource(visibleKeys, sourceById);

  if (viewMode === "table") {
    return <ServiceTableView groups={groups} />;
  }

  // If only personal services exist, skip section headers to preserve the
  // current flat-grid look-and-feel.
  const [firstGroup] = groups;
  if (groups.length === 1 && firstGroup && firstGroup.icon === "personal") {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {firstGroup.keys.map(({ keyInfo, source }) => (
          <KeyCard key={keyInfo.id} keyInfo={keyInfo} source={source} />
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {groups.map((group) => (
        <section key={group.key} className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2">
              {group.icon === "org" ? (
                <OrgAvatar
                  avatarUrl={group.avatarUrl}
                  displayName={group.title}
                  className="h-6 w-6 text-[0.625rem]"
                />
              ) : (
                <Globe className="h-4 w-4 text-muted-foreground" />
              )}
              <h3 className="text-[13px] font-semibold text-foreground">
                {group.title}
              </h3>
            </div>
            {group.role && <RoleBadge role={group.role} />}
            {group.subtitle && (
              <span className="text-xs text-muted-foreground">
                {group.subtitle}
              </span>
            )}
          </div>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {group.keys.map(({ keyInfo, source }) => (
              <KeyCard key={keyInfo.id} keyInfo={keyInfo} source={source} />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

function NyxIdApiKeysTab({
  createKeyOpen,
  onCreateKeyOpenChange,
  viewMode,
}: {
  readonly createKeyOpen?: boolean;
  readonly onCreateKeyOpenChange?: (open: boolean) => void;
  readonly viewMode: ViewMode;
}) {
  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <KeySquare className="h-4 w-4 text-muted-foreground" />
          <h3 className="text-[13px] font-semibold text-foreground">Agent Keys</h3>
        </div>
        <ApiKeyTable viewMode={viewMode} />
      </div>
      <ApiKeyUsageDashboard viewMode={viewMode} />
      <ApiKeyCreateDialog
        externalOpen={createKeyOpen}
        onExternalOpenChange={onCreateKeyOpenChange}
        hideTrigger
      />
    </div>
  );
}

function AddButton({
  tab,
  onAddService,
  onCreateKey,
}: {
  readonly tab: KeysTab;
  readonly onAddService: () => void;
  readonly onCreateKey: () => void;
}) {
  if (tab === "services") {
    return <AddCtaButton label="Add Service" onClick={onAddService} />;
  }
  return <AddCtaButton label="Create API Key" onClick={onCreateKey} />;
}

function AutoConnectedToggle({
  checked,
  onCheckedChange,
  count,
}: {
  readonly checked: boolean;
  readonly onCheckedChange: (checked: boolean) => void;
  readonly count: number;
}) {
  return (
    <div className="flex items-center gap-2">
      <Switch
        id="show-auto-connected"
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={count === 0}
      />
      <Label
        htmlFor="show-auto-connected"
        className="text-[12px] text-muted-foreground"
      >
        Show auto-connected ({count})
      </Label>
    </div>
  );
}

export function KeysPage() {
  const search: { tab?: string; slug?: string; action?: string } = useSearch({ strict: false });
  const navigate = useNavigate();
  const tab = parseTab(search.tab, KEYS_TABS, KEYS_TAB_DEFAULT);

  const [addServiceOpen, setAddServiceOpen] = useState(false);
  const [createKeyOpen, setCreateKeyOpen] = useState(false);
  const [showAutoConnected, setShowAutoConnected] = useState(false);
  const [servicesViewMode, setServicesViewMode] = useViewMode("keys-services");
  const [agentKeysViewMode, setAgentKeysViewMode] = useViewMode("keys-agent");
  const [pendingPrefillSlug, setPendingPrefillSlug] = useState<string | null>(null);
  const appliedSlugRef = useRef<string | null>(null);
  const appliedActionRef = useRef<string | null>(null);

  useEffect(() => {
    const slug = search.slug ?? null;
    if (slug) {
      if (appliedSlugRef.current === slug) return;
      appliedSlugRef.current = slug;
      setPendingPrefillSlug(slug);
      setAddServiceOpen(true);
      void navigate({
        to: "/keys",
        search: { tab: "services" },
        replace: true,
      });
      return;
    }

    const action: KeysAction | null = isValidTab(search.action, KEYS_ACTIONS)
      ? search.action
      : null;
    if (!action) return;
    if (appliedActionRef.current === action) return;
    appliedActionRef.current = action;

    if (action === "add-service") {
      setAddServiceOpen(true);
    } else if (action === "create-key") {
      setCreateKeyOpen(true);
    }
    void navigate({
      to: "/keys",
      search: { tab: search.tab },
      replace: true,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.slug, search.action]);

  // Clear the stashed slug AND reset the once-per-slug guard when
  // the dialog closes. Resetting `appliedSlugRef` lets a subsequent
  // `/keys?slug=<same-provider>` handoff auto-open the dialog again
  // — for example, two consecutive cli-pair retries for the same
  // catalog entry. Without the reset the second handoff's effect
  // short-circuits and the user lands on the keys list with no
  // dialog.
  function handleAddServiceOpenChange(next: boolean) {
    setAddServiceOpen(next);
    if (!next) {
      setPendingPrefillSlug(null);
      appliedSlugRef.current = null;
      appliedActionRef.current = null;
    }
  }

  function handleCreateKeyOpenChange(next: boolean) {
    setCreateKeyOpen(next);
    if (!next) {
      appliedActionRef.current = null;
    }
  }

  const { data: keys } = useKeys();
  const autoCount = (keys ?? []).filter((k) => k.auto_connected).length;

  function setTab(value: string) {
    void navigate({ to: "/keys", search: { tab: value }, replace: true });
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Services & Credentials"
        description="Manage your AI service credentials and agent keys."
      />

      <Tabs value={tab} onValueChange={setTab}>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between sm:gap-4">
          <TabsList className="min-w-0">
            <TabsTrigger value="services">External Services</TabsTrigger>
            <TabsTrigger value="nyxid">Agent Keys</TabsTrigger>
          </TabsList>
          <div className="flex shrink-0 items-center justify-between gap-4 sm:pb-1">
            {tab === "services" && (
              <AutoConnectedToggle
                checked={showAutoConnected}
                onCheckedChange={setShowAutoConnected}
                count={autoCount}
              />
            )}
            <ViewToggle
              viewMode={tab === "services" ? servicesViewMode : agentKeysViewMode}
              onViewModeChange={tab === "services" ? setServicesViewMode : setAgentKeysViewMode}
            />
            <AddButton
              tab={tab}
              onAddService={() => setAddServiceOpen(true)}
              onCreateKey={() => setCreateKeyOpen(true)}
            />
          </div>
        </div>

        <TabsContent value="services" className="mt-6">
          <ExternalServicesTab
            onAdd={() => setAddServiceOpen(true)}
            showAutoConnected={showAutoConnected}
            viewMode={servicesViewMode}
          />
        </TabsContent>

        <TabsContent value="nyxid" className="mt-6">
          <NyxIdApiKeysTab
            createKeyOpen={createKeyOpen}
            onCreateKeyOpenChange={handleCreateKeyOpenChange}
            viewMode={agentKeysViewMode}
          />
        </TabsContent>
      </Tabs>

      <AddKeyDialog
        open={addServiceOpen}
        onOpenChange={handleAddServiceOpenChange}
        prefillSlug={pendingPrefillSlug ?? undefined}
      />
    </div>
  );
}
