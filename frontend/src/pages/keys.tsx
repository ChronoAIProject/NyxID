import { ServiceConnectionTable } from "@/components/dashboard/service-connection-table";
import { ArchivedServiceHistory } from "@/components/dashboard/service-history";
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
import { Button, ButtonIcon } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { KeySquare, Terminal, RefreshCw, Shield } from "lucide-react";
import { MagicKeyIcon } from "@/components/icons/empty-state";
import {
  ViewToggle,
  useViewMode,
  type ViewMode,
} from "@/components/shared/view-toggle";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";
import { ApiKeyUsageDashboard } from "@/components/dashboard/api-key-usage-dashboard";
import { ServicePoolsTab } from "@/components/dashboard/service-pools-tab";
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
  )
    return false;
  const effectiveStatus = keyInfo.connection_status ?? keyInfo.status;
  if (
    !keyInfo.credential_missing &&
    !RECONNECTABLE_STATUSES.has(effectiveStatus)
  ) {
    return false;
  }
  return (
    keyInfo.credential_type === "oauth2" ||
    keyInfo.auth_method === "oauth2" ||
    keyInfo.auth_method === "oidc"
  );
}

function reconnectLabel(status: string): string {
  return status === "pending_auth" ? "Continue authentication" : "Reconnect";
}

function ConnectionReconnect({
  connection,
  onReconnect,
}: {
  readonly connection: KeyInfo;
  readonly onReconnect?: (key: KeyInfo) => void;
}) {
  if (
    !onReconnect ||
    !isReconnectableKey(connection, connection.credential_source)
  )
    return null;
  return (
    <Button
      size="sm"
      variant="link"
      className="mt-1 flex h-auto p-0 text-[11px]"
      onClick={() => onReconnect(connection)}
    >
      <RefreshCw className="size-3" />
      {reconnectLabel(connection.status)}
    </Button>
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
      <ErrorBanner
        message="Failed to load services. Please try again."
        onRetry={refetch}
      />
    );
  }

  if (!keys?.length) return <ServicesEmptyState onAdd={onAdd} />;

  return (
    <GroupedServiceCards
      keys={keys.map((keyInfo) => ({
        ...keyInfo,
        credential_source:
          keyInfo.credential_source ?? sourceById.get(keyInfo.id),
      }))}
      catalog={catalog}
      actions={(compact) => (
        <AddCtaButton
          label="Connect Service"
          onClick={onAdd}
          compact={compact}
          compactLabel="Connect"
        />
      )}
      renderTable={
        viewMode === "table"
          ? (filteredKeys) => (
              <div className="overflow-hidden rounded-xl border border-border bg-card">
                <ServiceConnectionTable
                  connections={filteredKeys}
                  serviceName="All services"
                  renderActions={(key) => (
                    <ConnectionReconnect
                      connection={key}
                      onReconnect={onReconnect}
                    />
                  )}
                />
              </div>
            )
          : undefined
      }
      renderConnectionActions={(keyInfo) => (
        <ConnectionReconnect connection={keyInfo} onReconnect={onReconnect} />
      )}
    />
  );
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
            <ButtonIcon>
              <Terminal className="h-3 w-3" />
            </ButtonIcon>
            Start Setup
          </Button>
        </CardContent>
      </Card>
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <KeySquare className="h-4 w-4 text-muted-foreground" />
          <h3 className="text-[13px] font-semibold text-foreground">
            Agent Keys
          </h3>
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

export function KeysPage() {
  const search: {
    tab?: string;
    slug?: string;
    action?: string;
    service?: string;
    view?: string;
  } = useSearch({ strict: false });
  const navigate = useNavigate();
  const tab = parseTab(search.tab, KEYS_TABS, KEYS_TAB_DEFAULT);
  const previewActive = Boolean(
    RoutingPreview &&
    (search.view === "routing" || import.meta.env.VITE_ROUTING_PREVIEW === "1"),
  );

  const [addServiceOpen, setAddServiceOpen] = useState(false);
  const [createPoolOpen, setCreatePoolOpen] = useState(false);
  const [createKeyOpen, setCreateKeyOpen] = useState(false);
  const [createKeySetupMode, setCreateKeySetupMode] = useState(false);
  const [initialSetupServiceId, setInitialSetupServiceId] = useState<
    string | null
  >(null);
  const [servicesViewMode, setServicesViewMode] = useViewMode("keys-services");
  const [agentKeysViewMode, setAgentKeysViewMode] = useViewMode("keys-agent");
  // Shared query with ExternalServicesTab; only decides header CTA placement.
  const { data: pageKeys } = useKeys();
  const [pendingPrefillSlug, setPendingPrefillSlug] = useState<string | null>(
    null,
  );
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
    void navigate({
      to: "/keys",
      search: { tab: value, ...(previewActive ? { view: "routing" } : {}) },
      replace: true,
    });
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Services & Credentials"
        description="Manage your AI service credentials and agent keys."
        actions={
          import.meta.env.DEV && !previewActive ? (
            <Button variant="outline" asChild>
              <Link to="/keys" search={{ view: "routing" }}>
                Routing preview
              </Link>
            </Button>
          ) : undefined
        }
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
                viewMode={
                  tab === "services" ? servicesViewMode : agentKeysViewMode
                }
                onViewModeChange={
                  tab === "services"
                    ? setServicesViewMode
                    : setAgentKeysViewMode
                }
              />
            )}
            {/* Services keep Connect Service inside the sticky filter toolbar;
                the empty state has no toolbar, so the header button stays. */}
            {(tab === "nyxid" || (tab === "services" && !pageKeys?.length)) && (
              <AddButton
                tab={tab}
                onAddService={() => setAddServiceOpen(true)}
                onCreatePool={() => setCreatePoolOpen(true)}
                onCreateKey={() => setCreateKeyOpen(true)}
              />
            )}
          </div>
        </div>

        <TabsContent value="services" className="mt-6">
          <CodexConnectionSection />
          {previewActive && RoutingPreview ? (
            <Suspense fallback={<Skeleton className="h-96 w-full" />}>
              <RoutingPreview
                actions={(compact) => (
                  <AddCtaButton
                    label="Connect Service"
                    onClick={() => setAddServiceOpen(true)}
                    compact={compact}
                    compactLabel="Connect"
                  />
                )}
                renderConnectionActions={(connection) => (
                  <ConnectionReconnect
                    connection={connection}
                    onReconnect={(keyInfo) => {
                      setReconnectKey(keyInfo);
                      setAddServiceOpen(true);
                    }}
                  />
                )}
              />
            </Suspense>
          ) : (
            <ExternalServicesTab
              onAdd={() => setAddServiceOpen(true)}
              onReconnect={(keyInfo) => {
                setReconnectKey(keyInfo);
                setAddServiceOpen(true);
              }}
              viewMode={servicesViewMode}
            />
          )}
          <ArchivedServiceHistory />
        </TabsContent>

        <TabsContent value="pools" className="mt-6">
          <ServicePoolsTab
            layout="cards"
            createOpen={createPoolOpen}
            onCreateOpenChange={setCreatePoolOpen}
          />
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
