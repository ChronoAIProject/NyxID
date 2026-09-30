import { ServiceConnectionTable } from "@/components/dashboard/service-connection-table";
import { canEditConnection } from "@/lib/connection-access";
import { ServiceAuthorshipFooter, ArchivedServiceHistory } from "@/components/dashboard/service-history";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearch, useNavigate } from "@tanstack/react-router";
import { useKeys, useCatalog } from "@/hooks/use-keys";
import { GroupedServiceCards } from "@/components/dashboard/grouped-service-cards";
import { useUserServices } from "@/hooks/use-user-services";
import { PageHeader } from "@/components/shared/page-header";
import { CodexConnectionSection } from "@/components/providers/codex-connection";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { TeachingEmptyState } from "@/components/shared/teaching-empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Globe,
  KeySquare,
  Server,
  Terminal,
  RefreshCw,
  Shield,
} from "lucide-react";
import { MagicKeyIcon } from "@/components/icons/empty-state";
import { useNodes } from "@/hooks/use-nodes";
import { ViewToggle, useViewMode, type ViewMode } from "@/components/shared/view-toggle";
import { ServiceIcon } from "@/components/service-icon";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";
import { ApiKeyUsageDashboard } from "@/components/dashboard/api-key-usage-dashboard";
import { ServicePoolsTab } from "@/components/dashboard/service-pools-tab";
import { RoleBadge } from "@/components/orgs/role-badge";
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
    case "online":
      return "success";
    case "expired":
    case "inaccessible":
    case "draining":
      return "secondary";
    case "revoked":
    case "failed":
    case "refresh_failed":
    case "offline":
    case "node_deleted":
    case "unknown":
      return "destructive";
    default:
      return "secondary";
  }
}

interface KeyCardProps {
  readonly keyInfo: KeyInfo;
  /** Credential provenance; missing ownership hides configuration. */
  readonly source: CredentialSource | undefined;
}

const RECONNECTABLE_STATUSES = new Set([
  "pending_auth",
  "refresh_failed",
  "failed",
  "expired",
]);

function isNonAdminOrgSource(source: CredentialSource | undefined): boolean {
  return source?.type === "org" && source.role !== "admin";
}

function isReconnectableKey(
  keyInfo: KeyInfo,
  source: CredentialSource | undefined,
): boolean {
  if (
    keyInfo.auto_connected ||
    isNonAdminOrgSource(source) ||
    (source?.type === "org" && !source.allowed)
  ) return false;
  const effectiveStatus = keyInfo.connection_status ?? keyInfo.status;
  if (!keyInfo.credential_missing && !RECONNECTABLE_STATUSES.has(effectiveStatus)) {
    return false;
  }
  return (
    keyInfo.credential_type === "oauth2" ||
    keyInfo.auth_method === "oauth2" ||
    keyInfo.auth_method === "oidc"
  );
}

function reconnectLabel(status: string): string {
  return status === "pending_auth"
    ? "Continue authentication"
    : "Reconnect";
}

function ConnectionReconnect({ connection, onReconnect }: {
  readonly connection: KeyInfo;
  readonly onReconnect?: (key: KeyInfo) => void;
}) {
  if (!onReconnect || !isReconnectableKey(connection, connection.credential_source)) return null;
  return (
    <Button size="sm" variant="link" className="mt-1 flex h-auto p-0 text-[11px]" onClick={() => onReconnect(connection)}>
      <RefreshCw className="size-3" />
      {reconnectLabel(connection.status)}
    </Button>
  );
}

function KeyCardContent({
  keyInfo,
  source,
  onReconnect,
}: KeyCardProps & {
  readonly onReconnect?: (keyInfo: KeyInfo) => void;
}) {
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
  const endpointUrl = keyInfo.endpoint_url ?? "";
  const displayUrl = !canEditConnection({ ...keyInfo, credential_source: source })
    ? (keyInfo.auto_connected ? "Platform managed" : "Editors only")
    : isSsh
      ? `${keyInfo.ssh_host ?? "unknown"}:${keyInfo.ssh_port ?? 22}`
      : endpointUrl.length > 50
        ? `${endpointUrl.slice(0, 50)}...`
        : endpointUrl;

  const isOrgInherited = source?.type === "org";
  // Viewers and out-of-scope members see the card with reduced opacity.
  const isBlocked = source?.type === "org" && !source.allowed;
  // Members can USE the credential (allowed=true) but cannot MODIFY it.
  const isReadOnly =
    source?.type === "org" && source.allowed && source.role !== "admin";

  const displayStatus = keyInfo.connection_status === "expired"
    ? "expired"
    : keyInfo.node_id && keyInfo.node_status
    ? (keyInfo.node_status === "unknown" ? "node_deleted" : keyInfo.node_status)
    : keyInfo.status;

  const displayStatusLabel =
    displayStatus === "node_deleted"
      ? "Node Deleted"
      : displayStatus.charAt(0).toUpperCase() + displayStatus.slice(1);
  const showReconnect = onReconnect && isReconnectableKey(keyInfo, source);
  const autoAuthLabel = keyInfo.auth_method === "none"
    ? "No auth required"
    : "Platform managed";

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
        <div className="flex items-start gap-3 min-w-0">
          <ServiceIcon
            slug={keyInfo.catalog_service_slug ?? keyInfo.slug}
            iconUrl={keyInfo.icon_url}
            size="md"
            className="mt-0.5"
          />
          <div className="min-w-0 flex-1">
            <p className="truncate text-[12px] font-medium text-foreground">
              {keyInfo.label}
            </p>
            {keyInfo.catalog_service_name && (
              <p className="truncate text-xs text-muted-foreground">
                {keyInfo.catalog_service_name}
              </p>
            )}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {isOrgInherited && (
            <Badge variant="info">{source.org_name}</Badge>
          )}
          {isOrgInherited && (
            <RoleBadge role={source.role} />
          )}
          {isBlocked && (
            <Badge variant="secondary">Read-Only</Badge>
          )}
          {isReadOnly && !isBlocked && (
            <Badge variant="secondary">View-Only</Badge>
          )}
          {keyInfo.admin_only && (
            <Badge variant="secondary">Admin-only</Badge>
          )}
          <Badge variant={keyInfo.is_active ? statusVariant(displayStatus) : "secondary"}>
            {keyInfo.is_active ? displayStatusLabel : "Disabled"}
          </Badge>
          {keyInfo.credential_missing && (
            <Badge variant="warning">Credential Missing</Badge>
          )}
          {isSsh && <Badge variant="secondary">SSH</Badge>}
          {(keyInfo.auto_connected ||
            isSsh ||
            (keyInfo.credential_type !== "oauth2" &&
              keyInfo.credential_type !== "api_key")) && (
            <Badge variant="secondary">
              {keyInfo.auto_connected
                ? autoAuthLabel
                : isSsh
                  ? hasSshCertificateAuth
                    ? "certificate"
                    : "ssh tunnel"
                  : keyInfo.credential_type}
            </Badge>
          )}
          {/* Routing pill — moved to top so it aligns across cards.
              When routed via a node, the badge becomes a real Link so the
              user can jump straight to the node detail page (deferred Wave B
              cleanup, ships with C.1 canon sweep). */}
          {nodeName && keyInfo.node_id ? (
            <Link
              to="/nodes/$nodeId"
              params={{ nodeId: keyInfo.node_id }}
              onClick={(e) => e.stopPropagation()}
              className="inline-flex"
            >
              <Badge variant="secondary" className="cursor-pointer transition-colors hover:bg-muted/70">
                → {nodeName}
              </Badge>
            </Link>
          ) : (
            <Badge variant="secondary">Direct</Badge>
          )}
          {keyInfo.auto_connected && (
            <Badge variant="secondary">
              {keyInfo.source_app_name
                ? `Via ${keyInfo.source_app_name}`
                : "Auto-connected"}
            </Badge>
          )}

        </div>

        {showReconnect && (
          <Button
            variant="outline"
            className="w-fit"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onReconnect(keyInfo);
            }}
          >
            <ButtonIcon><RefreshCw className="h-3 w-3" /></ButtonIcon>
            {reconnectLabel(keyInfo.status)}
          </Button>
        )}

        <div className="mt-auto flex min-w-0 items-end justify-between gap-3">
          <div className="min-w-0 flex-1 space-y-1.5 text-xs text-muted-foreground">
            <div className="flex min-w-0 items-center gap-1.5">
              {isSsh ? (
                <Terminal className="h-3 w-3 shrink-0" />
              ) : (
                <Globe className="h-3 w-3 shrink-0" />
              )}
              <span className="truncate">{displayUrl}</span>
            </div>
            <div className="flex min-w-0 items-center gap-1.5">
              <Server className="h-3 w-3 shrink-0" />
              <span className="truncate">
                {isSsh ? keyInfo.slug : `/proxy/s/${keyInfo.slug}`}
              </span>
            </div>
          </div>
          <ServiceAuthorshipFooter
            authorship={keyInfo.authorship}
            className="mt-0 max-w-[60%]"
          />
        </div>
      </CardContent>
    </Card>
  );
}

function KeyCard({
  keyInfo,
  source,
  onReconnect,
}: KeyCardProps & {
  readonly onReconnect?: (keyInfo: KeyInfo) => void;
}) {
  // Connection metadata and history stay navigable; the detail page gates configuration.
  return (
    <Link to="/keys/$keyId" params={{ keyId: keyInfo.id }} className="h-full">
      <KeyCardContent
        keyInfo={keyInfo}
        source={source}
        onReconnect={onReconnect}
      />
    </Link>
  );
}

function ServicesEmptyState({ onAdd }: { readonly onAdd: () => void }) {
  return (
    <TeachingEmptyState
      icon={MagicKeyIcon}
      title="No AI services yet"
      description="Connect a downstream service (OpenAI, GitHub, Anthropic, etc.) so your AI agents can call it through NyxID without ever seeing the raw key."
      primaryCta={{ label: "Add your first service", onClick: onAdd }}
    />
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
  onReconnect,
  viewMode,
}: {
  readonly onAdd: () => void;
  readonly onReconnect: (keyInfo: KeyInfo) => void;
  readonly viewMode: ViewMode;
}) {
  const { data: keys, isLoading, error, refetch } = useKeys();
  // user-services carries credential_source for both personal and
  // org-inherited items. When the backend augments /keys directly in a
  // future change, the `credential_source` field on KeyInfo will take
  // precedence and this call becomes a no-op.
  const { data: userServices } = useUserServices();
  const { data: catalog } = useCatalog();

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

  if (!keys?.length) return <ServicesEmptyState onAdd={onAdd} />;

  return <GroupedServiceCards
    keys={keys.map((keyInfo) => ({
      ...keyInfo,
      credential_source: keyInfo.credential_source ?? sourceById.get(keyInfo.id),
    }))}
    catalog={catalog}
    actions={(compact) => <AddCtaButton label="Connect Service" onClick={onAdd} compact={compact} compactLabel="Connect" />}
    renderTable={viewMode === "table" ? (filteredKeys) => (
      <div className="overflow-hidden rounded-xl border border-border bg-card">
        <ServiceConnectionTable connections={filteredKeys} serviceName="All services" renderActions={(key) => <ConnectionReconnect connection={key} onReconnect={onReconnect} />} />
      </div>
    ) : undefined}
    renderConnectionActions={(keyInfo) => <ConnectionReconnect
      connection={keyInfo}
      onReconnect={onReconnect}
    />}
  />;
}

function NyxIdApiKeysTab({
  createKeyOpen,
  onCreateKeyOpenChange,
  onSetupAgent,
  createKeySetupMode,
  initialSetupServiceId,
  viewMode,
}: {
  readonly createKeyOpen?: boolean;
  readonly onCreateKeyOpenChange?: (open: boolean) => void;
  readonly onSetupAgent: () => void;
  readonly createKeySetupMode: boolean;
  readonly initialSetupServiceId: string | null;
  readonly viewMode: ViewMode;
}) {
  return (
    <div className="space-y-6">
      <Card>
        <CardContent className="flex flex-col gap-4 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-primary/25 bg-primary/10">
              <Shield className="h-4 w-4 text-primary" />
            </div>
            <div className="space-y-1">
              <h3 className="text-[13px] font-semibold text-foreground">
                Set up an isolated AI agent
              </h3>
              <p className="max-w-2xl text-xs text-muted-foreground">
                Create or select an Agent Key, choose allowed services, add
                credential bindings only when needed, then copy a verification
                command that proves the key is scoped.
              </p>
            </div>
          </div>
          <Button className="shrink-0" onClick={onSetupAgent}>
            <ButtonIcon><Terminal className="h-3 w-3" /></ButtonIcon>
            Start Setup
          </Button>
        </CardContent>
      </Card>
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
        setupMode={createKeySetupMode}
        initialServiceId={initialSetupServiceId}
      />
    </div>
  );
}

function AddButton({
  tab,
  onAddService,
  onCreatePool,
  onCreateKey,
}: {
  readonly tab: KeysTab;
  readonly onAddService: () => void;
  readonly onCreatePool: () => void;
  readonly onCreateKey: () => void;
}) {
  if (tab === "services") {
    return <AddCtaButton label="Connect Service" onClick={onAddService} />;
  }
  if (tab === "pools") {
    return <AddCtaButton label="Create Pool" onClick={onCreatePool} />;
  }
  return <AddCtaButton label="Create API Key" onClick={onCreateKey} />;
}

const RoutingPreview = import.meta.env.DEV
  ? lazy(() => import("@/components/dashboard/service-routing-preview"))
  : null;
const PoolRoutingPreview = import.meta.env.DEV
  ? lazy(() => import("@/components/dashboard/service-pool-routing-preview"))
  : null;

export function KeysPage() {
  const search: { tab?: string; slug?: string; action?: string; service?: string; view?: string } = useSearch({ strict: false });
  const navigate = useNavigate();
  const tab = parseTab(search.tab, KEYS_TABS, KEYS_TAB_DEFAULT);
  const previewActive = Boolean(RoutingPreview && (search.view === "routing" || import.meta.env.VITE_ROUTING_PREVIEW === "1"));

  const [addServiceOpen, setAddServiceOpen] = useState(false);
  const [createPoolOpen, setCreatePoolOpen] = useState(false);
  const [createKeyOpen, setCreateKeyOpen] = useState(false);
  const [createKeySetupMode, setCreateKeySetupMode] = useState(false);
  const [initialSetupServiceId, setInitialSetupServiceId] = useState<string | null>(null);
  const [servicesViewMode, setServicesViewMode] = useViewMode("keys-services");
  const [agentKeysViewMode, setAgentKeysViewMode] = useViewMode("keys-agent");
  // Shared query with ExternalServicesTab; only decides header CTA placement.
  const { data: pageKeys } = useKeys();
  const [pendingPrefillSlug, setPendingPrefillSlug] = useState<string | null>(null);
  const [reconnectKey, setReconnectKey] = useState<KeyInfo | null>(null);
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
      setCreateKeySetupMode(false);
      setInitialSetupServiceId(null);
      setCreateKeyOpen(true);
    } else if (action === "setup-agent") {
      setCreateKeySetupMode(true);
      setInitialSetupServiceId(search.service ?? null);
      setCreateKeyOpen(true);
    }
    void navigate({
      to: "/keys",
      search: { tab: search.tab },
      replace: true,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.slug, search.action, search.service]);

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
      setReconnectKey(null);
      appliedSlugRef.current = null;
      appliedActionRef.current = null;
    }
  }

  function handleCreateKeyOpenChange(next: boolean) {
    setCreateKeyOpen(next);
    if (!next) {
      setCreateKeySetupMode(false);
      setInitialSetupServiceId(null);
      appliedActionRef.current = null;
    }
  }

  function setTab(value: string) {
    void navigate({ to: "/keys", search: { tab: value, ...(previewActive ? { view: "routing" } : {}) }, replace: true });
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Services & Credentials"
        description="Manage your AI service credentials and agent keys."
        actions={import.meta.env.DEV && !previewActive ? <Button variant="outline" asChild><Link to="/keys" search={{ view: "routing" }}>Routing preview</Link></Button> : undefined}
      />

      <Tabs value={tab} onValueChange={setTab}>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between sm:gap-4">
          <TabsList className="min-w-0">
            <TabsTrigger value="services">External Services</TabsTrigger>
            <TabsTrigger value="pools">Service Pools</TabsTrigger>
            <TabsTrigger value="nyxid">Agent Keys</TabsTrigger>
          </TabsList>
          <div className="flex shrink-0 items-center justify-between gap-4 sm:pb-1">
            {tab !== "pools" && !(tab === "services" && previewActive) && (
              <ViewToggle
                viewMode={tab === "services" ? servicesViewMode : agentKeysViewMode}
                onViewModeChange={tab === "services" ? setServicesViewMode : setAgentKeysViewMode}
              />
            )}
            {/* Services keep Connect Service inside the sticky filter toolbar;
                the empty state has no toolbar, so the header button stays. */}
            {(tab !== "services" || !pageKeys?.length) && !(previewActive && tab === "pools") && <AddButton
              tab={tab}
              onAddService={() => setAddServiceOpen(true)}
              onCreatePool={() => setCreatePoolOpen(true)}
              onCreateKey={() => setCreateKeyOpen(true)}
            />}
          </div>
        </div>

        <TabsContent value="services" className="mt-6">
          <CodexConnectionSection />
          {previewActive && RoutingPreview ? (
            <Suspense fallback={<Skeleton className="h-96 w-full" />}>
              <RoutingPreview
                actions={(compact) => <AddCtaButton label="Connect Service" onClick={() => setAddServiceOpen(true)} compact={compact} compactLabel="Connect" />}
                renderConnectionActions={(connection) => (
                  <ConnectionReconnect connection={connection} onReconnect={(keyInfo) => {
                    setReconnectKey(keyInfo);
                    setAddServiceOpen(true);
                  }} />
                )} />
            </Suspense>
          ) : <ExternalServicesTab
            onAdd={() => setAddServiceOpen(true)}
            onReconnect={(keyInfo) => {
              setReconnectKey(keyInfo);
              setAddServiceOpen(true);
            }}
            viewMode={servicesViewMode}
          />}
          <ArchivedServiceHistory />
        </TabsContent>

        <TabsContent value="pools" className="mt-6">
          {previewActive && PoolRoutingPreview ? (
            <Suspense fallback={<Skeleton className="h-96 w-full" />}>
              <PoolRoutingPreview renderConnection={(candidate) => (
                <KeyCard keyInfo={candidate.key} source={candidate.source} />
              )} />
            </Suspense>
          ) : <ServicePoolsTab
            createOpen={createPoolOpen}
            onCreateOpenChange={setCreatePoolOpen}
          />}
        </TabsContent>

        <TabsContent value="nyxid" className="mt-6">
          <NyxIdApiKeysTab
            createKeyOpen={createKeyOpen}
            onCreateKeyOpenChange={handleCreateKeyOpenChange}
            onSetupAgent={() => {
              setCreateKeySetupMode(true);
              setInitialSetupServiceId(null);
              setCreateKeyOpen(true);
            }}
            createKeySetupMode={createKeySetupMode}
            initialSetupServiceId={initialSetupServiceId}
            viewMode={agentKeysViewMode}
          />
        </TabsContent>
      </Tabs>

      <AddKeyDialog
        open={addServiceOpen}
        onOpenChange={handleAddServiceOpenChange}
        prefillSlug={pendingPrefillSlug ?? undefined}
        reconnectKey={reconnectKey}
      />
    </div>
  );
}
