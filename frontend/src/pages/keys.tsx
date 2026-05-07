import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearch, useNavigate } from "@tanstack/react-router";
import { useKeys } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { PageHeader } from "@/components/shared/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Plus,
  Globe,
  KeyRound,
  Server,
  Router,
  Terminal,
  Zap,
  Building2,
  Lock,
} from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { useNodes } from "@/hooks/use-nodes";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ApiKeyTable } from "@/components/dashboard/api-key-table";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";
import { ApiKeyUsageDashboard } from "@/components/dashboard/api-key-usage-dashboard";
import { RoleBadge } from "@/components/orgs/role-badge";
import { OrgAvatar } from "@/components/orgs/org-avatar";
import type { KeyInfo } from "@/types/keys";
import type { CredentialSource } from "@/schemas/orgs";

type TabValue = "services" | "nyxid";

function statusVariant(
  status: string,
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "active":
      return "default";
    case "expired":
      return "secondary";
    case "revoked":
    case "failed":
    case "refresh_failed":
      return "destructive";
    default:
      return "outline";
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
      className={`transition-colors ${
        isBlocked
          ? "opacity-60"
          : "hover:border-primary/30 hover:bg-accent/30"
      }`}
      aria-disabled={isBlocked ? true : undefined}
    >
      <CardContent className="flex flex-col gap-3 p-5">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium text-foreground">
              {keyInfo.label}
            </p>
            {keyInfo.catalog_service_name && (
              <p className="text-xs text-muted-foreground">
                {keyInfo.catalog_service_name}
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {isOrgInherited && (
              <Badge variant="info" className="gap-1">
                <Building2 className="h-3 w-3" />
                Org
              </Badge>
            )}
            {isBlocked && (
              <Badge variant="secondary" className="gap-1">
                <Lock className="h-3 w-3" />
                Read-only
              </Badge>
            )}
            {isReadOnly && !isBlocked && (
              <Badge variant="secondary" className="gap-1">
                <Lock className="h-3 w-3" />
                View-only
              </Badge>
            )}
            {keyInfo.auto_connected && (
              <Badge variant="secondary">
                {keyInfo.source_app_name
                  ? `Via ${keyInfo.source_app_name}`
                  : "Auto-connected"}
              </Badge>
            )}
            {isSsh && <Badge variant="outline">SSH</Badge>}
            <Badge variant={statusVariant(keyInfo.status)}>
              {keyInfo.status}
            </Badge>
            {!keyInfo.is_active && <Badge variant="outline">Inactive</Badge>}
          </div>
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
          <div className="flex items-center gap-1.5">
            {isSsh ? (
              <Terminal className="h-3 w-3 shrink-0" />
            ) : (
              <Globe className="h-3 w-3 shrink-0" />
            )}
            <span className="truncate">{displayUrl}</span>
          </div>
          <div className="flex items-center gap-1.5">
            {keyInfo.auto_connected ? (
              <Zap className="h-3 w-3 shrink-0" />
            ) : (
              <KeyRound className="h-3 w-3 shrink-0" />
            )}
            <span>
              {keyInfo.auto_connected
                ? "No auth required"
                : isSsh
                  ? hasSshCertificateAuth
                    ? "certificate"
                    : "ssh tunnel"
                  : keyInfo.credential_type}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <Server className="h-3 w-3 shrink-0" />
            <span className="font-mono">
              {isSsh ? keyInfo.slug : `/proxy/s/${keyInfo.slug}`}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
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
    <Link to="/keys/$keyId" params={{ keyId: keyInfo.id }}>
      <KeyCardContent keyInfo={keyInfo} source={source} />
    </Link>
  );
}

interface ServiceGroup {
  readonly key: string;
  readonly title: string;
  readonly subtitle: string | null;
  readonly role: "admin" | "member" | "viewer" | null;
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
    title: "My services",
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
    <Card>
      <CardContent className="flex flex-col items-center gap-4 py-16">
        <div className="flex h-14 w-14 items-center justify-center rounded-full border border-border">
          <KeyRound className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="max-w-md space-y-2 text-center">
          <p className="text-sm font-medium">No AI services yet</p>
          <p className="text-xs text-muted-foreground">
            Connect a downstream service (OpenAI, GitHub, Anthropic, etc.) so your
            AI agents can call it through NyxID without ever seeing the raw key.
          </p>
          <p className="text-xs text-muted-foreground">
            Connect a service <span className="font-medium">before</span> wiring
            up MCP &mdash; otherwise your AI agent will only see NyxID&apos;s{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-[0.7rem]">
              nyx__&hellip;
            </code>{" "}
            meta-tools and proxy requests will look broken.
          </p>
        </div>
        <Button size="sm" onClick={onAdd}>
          <Plus className="mr-2 h-4 w-4" />
          Add Service
        </Button>
      </CardContent>
    </Card>
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
}: {
  readonly onAdd: () => void;
  readonly showAutoConnected: boolean;
}) {
  const { data: keys, isLoading, error } = useKeys();
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
      <Card>
        <CardContent className="py-8 text-center text-sm text-destructive">
          Failed to load services. Please try again.
        </CardContent>
      </Card>
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
                <KeyRound className="h-4 w-4 text-muted-foreground" />
              )}
              <h3 className="text-sm font-semibold text-foreground">
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

function NyxIdApiKeysTab() {
  return (
    <div className="space-y-4">
      <ApiKeyUsageDashboard />
      <div className="flex justify-end">
        <ApiKeyCreateDialog />
      </div>
      <div className="rounded-xl border border-border">
        <ApiKeyTable />
      </div>
    </div>
  );
}

function AddButton({
  tab,
  onAddService,
}: {
  readonly tab: TabValue;
  readonly onAddService: () => void;
}) {
  if (tab === "services") {
    return (
      <Button size="sm" onClick={onAddService}>
        <Plus className="mr-2 h-4 w-4" />
        Add Service
      </Button>
    );
  }
  // "nyxid" tab -- the ApiKeyCreateDialog has its own trigger button
  return null;
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
  if (count === 0) return null;

  return (
    <div className="flex items-center gap-2">
      <Switch
        id="show-auto-connected"
        checked={checked}
        onCheckedChange={onCheckedChange}
      />
      <Label
        htmlFor="show-auto-connected"
        className="text-sm text-muted-foreground"
      >
        Show auto-connected ({count})
      </Label>
    </div>
  );
}

export function KeysPage() {
  const search: { tab?: string; slug?: string } = useSearch({ strict: false });
  const navigate = useNavigate();
  const rawTab = search.tab ?? "services";
  const tab: TabValue = rawTab === "nyxid" ? "nyxid" : "services";

  const [addServiceOpen, setAddServiceOpen] = useState(false);
  const [showAutoConnected, setShowAutoConnected] = useState(false);
  // Consume `?slug=X` once on mount and hold it in component state
  // so it survives the URL replace below. If we derived
  // `prefillSlug` from `search.slug` directly, the `navigate(...,
  // replace)` call would wipe it from the URL before `AddKeyDialog`
  // mounts and reads the prop, so the dialog's auto-select effect
  // would never see the slug and the user would land on the generic
  // catalog picker. Keeping it in state decouples "UI intent" from
  // "current URL" — the URL gets cleaned up immediately so a
  // refresh doesn't re-open the dialog, but the slug stays
  // addressable until the dialog closes and we reset it.
  const [pendingPrefillSlug, setPendingPrefillSlug] = useState<string | null>(null);
  const appliedSlugRef = useRef<string | null>(null);
  useEffect(() => {
    const slug = search.slug ?? null;
    if (!slug) return;
    if (appliedSlugRef.current === slug) return;
    appliedSlugRef.current = slug;
    setPendingPrefillSlug(slug);
    setAddServiceOpen(true);
    void navigate({
      to: "/keys",
      search: { tab: "services" },
      replace: true,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.slug]);

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
        title="AI Services"
        description="Manage your AI service credentials and agent keys."
        actions={
          <div className="flex items-center gap-4">
            {tab === "services" && (
              <AutoConnectedToggle
                checked={showAutoConnected}
                onCheckedChange={setShowAutoConnected}
                count={autoCount}
              />
            )}
            <AddButton
              tab={tab}
              onAddService={() => setAddServiceOpen(true)}
            />
          </div>
        }
      />

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="services">External Services</TabsTrigger>
          <TabsTrigger value="nyxid">Agent Keys</TabsTrigger>
        </TabsList>

        <TabsContent value="services" className="mt-6">
          <ExternalServicesTab
            onAdd={() => setAddServiceOpen(true)}
            showAutoConnected={showAutoConnected}
          />
        </TabsContent>

        <TabsContent value="nyxid" className="mt-6">
          <NyxIdApiKeysTab />
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
