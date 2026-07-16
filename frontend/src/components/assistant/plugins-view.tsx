import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Plus, Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ManageConnectionModal } from "@/components/assistant/manage-connection-modal";
import { ServiceIcon } from "@/components/service-icon";
import { useCatalog, useKeys } from "@/hooks/use-keys";
import {
  deriveConnectorItems,
  matchesPluginQuery,
  type ConnectorCardItem,
  type PluginKind,
} from "@/lib/assistant/plugins";
import {
  addSkill,
  listAddedSkillIds,
  SKILL_CATALOG,
  type SkillCatalogItem,
} from "@/lib/assistant/skills";

interface PluginCardShape {
  readonly id: string;
  readonly name: string;
  readonly initial: string;
  readonly iconSlug?: string | null;
  readonly category: string;
  readonly description: string;
  readonly meta: string;
  readonly added: boolean;
}

function PluginCard({
  item,
  addedBadge,
  addedMeta,
  addedAction,
  availableAction,
}: {
  readonly item: PluginCardShape;
  readonly addedBadge: "Connected" | "Installed";
  /** Extra mono note next to the added badge (e.g. "2 connections"). */
  readonly addedMeta?: string;
  readonly addedAction: React.ReactNode;
  readonly availableAction: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-border bg-card p-4 transition-colors hover:border-hairline-strong">
      <div className="flex items-start gap-2.5">
        <div className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-md bg-muted text-[13px] font-semibold text-muted-foreground">
          {item.iconSlug ? (
            <ServiceIcon slug={item.iconSlug} size="md" />
          ) : (
            item.initial
          )}
        </div>
        <div className="min-w-0">
          <p className="truncate text-[13px] font-medium text-foreground">
            {item.name}
          </p>
          <p className="text-[10px] uppercase tracking-[0.5px] text-text-tertiary">
            {item.category}
          </p>
        </div>
      </div>
      <p
        className="flex-1 text-[12px] text-muted-foreground line-clamp-4 break-words"
        title={item.description}
      >
        {item.description}
      </p>
      <div className="flex items-center justify-between gap-2">
        {item.added ? (
          <>
            <span className="flex min-w-0 items-center gap-2">
              <Badge variant="success">{addedBadge}</Badge>
              {addedMeta && (
                <span className="truncate font-mono text-[10px] text-text-tertiary">
                  {addedMeta}
                </span>
              )}
            </span>
            {addedAction}
          </>
        ) : (
          <>
            <span className="font-mono text-[10px] text-text-tertiary">
              {item.meta}
            </span>
            {availableAction}
          </>
        )}
      </div>
    </div>
  );
}

function AddYourOwnSkillCard() {
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-dashed border-border p-4">
      <div className="flex items-start gap-2.5">
        <div className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
          <Plus className="h-3.5 w-3.5" />
        </div>
        <div className="min-w-0">
          <p className="text-[13px] font-medium text-foreground">
            Add your own skill
          </p>
          <p className="text-[10px] uppercase tracking-[0.5px] text-text-tertiary">
            Skill
          </p>
        </div>
      </div>
      <p className="flex-1 text-[12px] text-muted-foreground">
        Register a skill from a Git repo or paste a SKILL.md - your assistant
        loads it on demand in every chat.
      </p>
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[10px] text-text-tertiary">
          git url or upload
        </span>
        <Button type="button" variant="outline" size="sm" disabled>
          Add skill
        </Button>
      </div>
    </div>
  );
}

function SectionHeading({ children }: { readonly children: React.ReactNode }) {
  return (
    <p className="mb-2.5 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
      {children}
    </p>
  );
}

function CardGrid({ children }: { readonly children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(248px,1fr))] gap-3">
      {children}
    </div>
  );
}

function LoadingGrid() {
  return (
    <div role="status" aria-label="Loading the plugin catalog">
      <CardGrid>
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-36 rounded-xl" />
        ))}
      </CardGrid>
    </div>
  );
}

function ConnectorCard({
  item,
  onManage,
}: {
  readonly item: ConnectorCardItem;
  readonly onManage: (keyId: string) => void;
}) {
  return (
    <PluginCard
      item={item}
      addedBadge="Connected"
      addedMeta={item.manageKeyId ? undefined : item.meta}
      addedAction={
        item.manageKeyId ? (
          // Single-connection service: manage in a compact modal in-place.
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onManage(item.manageKeyId as string)}
          >
            Manage
          </Button>
        ) : (
          // Multi-connection service: which credential is ambiguous, so send
          // to the Studio keys list (filtered) rather than a single-key modal.
          <Button asChild variant="outline" size="sm">
            <Link to="/keys">Manage</Link>
          </Button>
        )
      }
      availableAction={
        item.connectSlug ? (
          <Button asChild variant="primary" size="sm">
            <Link to="/keys" search={{ slug: item.connectSlug }}>
              Connect
            </Link>
          </Button>
        ) : null
      }
    />
  );
}

function ConnectorsTab({ query }: { readonly query: string }) {
  const keysQuery = useKeys();
  const catalogQuery = useCatalog();
  const [manageKeyId, setManageKeyId] = useState<string | null>(null);

  const items = useMemo(
    () => deriveConnectorItems(keysQuery.data ?? [], catalogQuery.data ?? []),
    [keysQuery.data, catalogQuery.data],
  );

  if (keysQuery.isLoading || catalogQuery.isLoading) return <LoadingGrid />;

  if (keysQuery.error || catalogQuery.error) {
    return (
      <ErrorBanner
        message="Failed to load the plugin catalog. Please try again."
        onRetry={() => {
          void keysQuery.refetch();
          void catalogQuery.refetch();
        }}
      />
    );
  }

  const searching = Boolean(query.trim());
  const added = items.added.filter((item) => matchesPluginQuery(item, query));
  const available = items.available.filter((item) =>
    matchesPluginQuery(item, query),
  );

  if (searching && added.length === 0 && available.length === 0) {
    return (
      <p className="py-14 text-center text-[11px] text-text-tertiary">
        No plugins match this search.
      </p>
    );
  }

  return (
    <>
      {(added.length > 0 || !searching) && (
        <>
          <SectionHeading>Added</SectionHeading>
          {added.length > 0 ? (
            <div className="mb-7">
              <CardGrid>
                {added.map((item) => (
                  <ConnectorCard
                    key={item.id}
                    item={item}
                    onManage={setManageKeyId}
                  />
                ))}
              </CardGrid>
            </div>
          ) : (
            <p className="mb-7 rounded-xl border border-dashed border-border px-4 py-6 text-center text-[12px] text-muted-foreground">
              No connected services yet. Connect one below and your assistant
              can call it through the NyxID proxy.
            </p>
          )}
        </>
      )}

      {available.length > 0 && (
        <>
          <SectionHeading>Available to add</SectionHeading>
          <CardGrid>
            {available.map((item) => (
              <ConnectorCard
                key={item.id}
                item={item}
                onManage={setManageKeyId}
              />
            ))}
          </CardGrid>
        </>
      )}

      {manageKeyId !== null && (
        <ManageConnectionModal
          keyId={manageKeyId}
          onClose={() => setManageKeyId(null)}
        />
      )}
    </>
  );
}

function SkillsTab({ query }: { readonly query: string }) {
  const [addedIds, setAddedIds] = useState<ReadonlySet<string>>(() =>
    listAddedSkillIds(),
  );

  const items = SKILL_CATALOG.map((skill) => toSkillCard(skill, addedIds)).filter(
    (item) => matchesPluginQuery(item, query),
  );
  const added = items.filter((item) => item.added);
  const available = items.filter((item) => !item.added);
  const searching = Boolean(query.trim());
  const showOwnSkillCard = !searching;

  if (searching && items.length === 0) {
    return (
      <p className="py-14 text-center text-[11px] text-text-tertiary">
        No plugins match this search.
      </p>
    );
  }

  function card(item: PluginCardShape) {
    return (
      <PluginCard
        key={item.id}
        item={item}
        addedBadge="Installed"
        addedAction={
          <Button type="button" variant="outline" size="sm" disabled>
            Manage
          </Button>
        }
        availableAction={
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => setAddedIds(addSkill(item.id))}
          >
            Install
          </Button>
        }
      />
    );
  }

  return (
    <>
      {added.length > 0 && (
        <>
          <SectionHeading>Added</SectionHeading>
          <div className="mb-7">
            <CardGrid>{added.map(card)}</CardGrid>
          </div>
        </>
      )}

      {(available.length > 0 || showOwnSkillCard) && (
        <>
          <SectionHeading>Available to add</SectionHeading>
          <CardGrid>
            {available.map(card)}
            {showOwnSkillCard && <AddYourOwnSkillCard />}
          </CardGrid>
        </>
      )}
    </>
  );
}

function toSkillCard(
  skill: SkillCatalogItem,
  addedIds: ReadonlySet<string>,
): PluginCardShape {
  return {
    id: skill.id,
    name: skill.name,
    initial: skill.initial,
    category: "Skill",
    description: skill.description,
    meta: `${skill.author} · ${skill.version}`,
    added: addedIds.has(skill.id),
  };
}

export function PluginsView() {
  const [tab, setTab] = useState<PluginKind>("connector");
  const [query, setQuery] = useState("");

  return (
    <div className="h-full min-h-0 overflow-y-auto overscroll-contain">
      <div className="px-5 pt-6 sm:px-8">
        <h1 className="text-[22px] font-bold tracking-[-0.03em] sm:text-[28px]">
          Plugins
        </h1>
        <p className="mt-1 max-w-2xl text-[12px] text-muted-foreground">
          Plugins are connectors to services, MCP servers, and data sources;
          skills teach your assistant new workflows. Installing provisions the
          endpoint, credential, and proxy route in one step.
        </p>
        <div className="mt-4 flex flex-wrap items-center gap-4">
          <label className="flex h-8 w-full max-w-[300px] items-center gap-2 rounded-lg border border-hairline px-3 transition-colors focus-within:border-hairline-strong">
            <Search className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search the catalog..."
              className="min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-text-tertiary"
            />
          </label>
          <div className="flex h-8" role="tablist" aria-label="Plugin type">
            {(
              [
                ["connector", "Connectors"],
                ["skill", "Skills"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={tab === value}
                onClick={() => setTab(value)}
                className={`border-b-2 px-3 text-[12px] transition-colors ${
                  tab === value
                    ? "border-primary font-medium text-foreground"
                    : "border-transparent text-text-tertiary hover:text-foreground"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="px-5 pb-10 pt-5 sm:px-8">
        {tab === "connector" ? (
          <ConnectorsTab query={query} />
        ) : (
          <SkillsTab query={query} />
        )}
      </div>
    </div>
  );
}
