import { useEffect, useMemo, useState } from "react";
import { ArrowRight, Plus, RefreshCw, Search } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
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

/**
 * Every card in the grid renders at this exact box, whatever its description
 * length: header (34px tile) + a 3-line description + the action row, inside
 * `p-4`. The description is the only elastic block (`flex-1` + `line-clamp-3`),
 * so short and empty descriptions reserve the same space instead of collapsing
 * the card. `LoadingGrid` skeletons use the same height.
 */
const CARD_HEIGHT = "h-[168px]";

function PluginTile({
  children,
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <div className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-md bg-muted text-[13px] font-semibold text-muted-foreground">
      {children}
    </div>
  );
}

/**
 * A catalog card with exactly one call to action: the card itself. Clicking
 * anywhere runs the primary action — Connect for an unconnected service,
 * Manage for a connected one — rather than staging a detail view in front of
 * it. `actionLabel` is the card's own affordance; it is deliberately a label,
 * not a nested button, so the whole surface stays a single hit target.
 *
 * A card with no `onActivate` (an installed skill, whose management isn't
 * built yet) renders inert: not focusable, no pointer affordance.
 */
function PluginCard({
  item,
  addedBadge,
  addedMeta,
  actionLabel,
  onActivate,
}: {
  readonly item: PluginCardShape;
  readonly addedBadge: "Connected" | "Installed";
  /** Extra mono note next to the added badge (e.g. "2 connections"). */
  readonly addedMeta?: string;
  readonly actionLabel: string;
  readonly onActivate?: () => void;
}) {
  const interactive = Boolean(onActivate);

  return (
    <div
      {...(interactive
        ? {
            role: "button",
            tabIndex: 0,
            "aria-label": `${actionLabel} ${item.name}`,
            onClick: onActivate,
            onKeyDown: (event: React.KeyboardEvent) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onActivate?.();
              }
            },
          }
        : {})}
      // The clamped description is the only truncated text on the card, and
      // the card no longer expands — the native tooltip keeps the full copy
      // reachable without a second surface.
      title={item.description}
      className={`group flex ${CARD_HEIGHT} flex-col gap-2.5 rounded-xl border border-border bg-card p-4 text-left transition-colors ${
        interactive ? "cursor-pointer hover:border-hairline-strong" : ""
      }`}
    >
      <div className="flex items-start gap-2.5">
        <PluginTile>
          {item.iconSlug ? (
            <ServiceIcon slug={item.iconSlug} size="md" />
          ) : (
            item.initial
          )}
        </PluginTile>
        <div className="min-w-0">
          <p className="truncate text-[13px] font-medium text-foreground">
            {item.name}
          </p>
          <p className="text-[10px] uppercase tracking-[0.5px] text-text-tertiary">
            {item.category}
          </p>
        </div>
      </div>
      {/* The spacer absorbs the card's slack so the action row stays pinned to
          the bottom; the clamp lives on the text itself, or `flex-1` would
          stretch the box past three lines and clip a fourth mid-glyph. */}
      <div className="min-h-0 flex-1">
        <p className="break-words text-[12px] leading-[17px] text-muted-foreground line-clamp-3">
          {item.description}
        </p>
      </div>
      <div className="flex shrink-0 items-center justify-between gap-2">
        {item.added ? (
          <span className="flex min-w-0 items-center gap-2">
            <Badge variant="success">{addedBadge}</Badge>
            {addedMeta && (
              <span className="truncate font-mono text-[10px] text-text-tertiary">
                {addedMeta}
              </span>
            )}
          </span>
        ) : (
          <span className="font-mono text-[10px] text-text-tertiary">
            {item.meta}
          </span>
        )}
        <span
          aria-hidden
          className={`flex shrink-0 items-center gap-1 text-[12px] transition-colors ${
            interactive
              ? "text-muted-foreground group-hover:text-foreground"
              : "text-text-tertiary"
          }`}
        >
          {actionLabel}
          {interactive && <ArrowRight className="h-3 w-3" />}
        </span>
      </div>
    </div>
  );
}

function AddYourOwnSkillCard() {
  return (
    <div
      className={`flex ${CARD_HEIGHT} flex-col gap-2.5 rounded-xl border border-dashed border-border p-4`}
    >
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
      <p className="flex-1 text-[12px] leading-[17px] text-muted-foreground line-clamp-3">
        Register a skill from a Git repo or paste a SKILL.md - your assistant
        loads it on demand in every chat.
      </p>
      <div className="flex shrink-0 items-center justify-between gap-2">
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
          <Skeleton key={i} className={`${CARD_HEIGHT} rounded-xl`} />
        ))}
      </CardGrid>
    </div>
  );
}

function ConnectorCard({
  item,
  onManage,
  onConnect,
}: {
  readonly item: ConnectorCardItem;
  readonly onManage: (cardId: string) => void;
  readonly onConnect: (slug: string) => void;
}) {
  const connectSlug = item.connectSlug;
  return (
    <PluginCard
      item={item}
      addedBadge="Connected"
      addedMeta={(item.manageKeyIds?.length ?? 0) > 1 ? item.meta : undefined}
      actionLabel={item.added ? "Manage" : "Connect"}
      onActivate={
        item.added
          ? () => onManage(item.id)
          : connectSlug
            ? () => onConnect(connectSlug)
            : undefined
      }
    />
  );
}

function ConnectorsTab({ query }: { readonly query: string }) {
  const keysQuery = useKeys();
  const catalogQuery = useCatalog();
  const [manageCardId, setManageCardId] = useState<string | null>(null);
  const [connectSlug, setConnectSlug] = useState<string | null>(null);
  // Settled result of a managed-popup OAuth connect. The popup's "view result"
  // action hands back the key it authorized, and the grid opens the same
  // manage modal the Added cards use. The label rides along because
  // `connectSlug` is cleared as the dialog closes and the freshly minted key
  // has not landed in `items` yet.
  const [popupResult, setPopupResult] = useState<{
    readonly keyId: string;
    readonly name: string;
    readonly iconSlug: string | null;
  } | null>(null);

  const items = useMemo(
    () => deriveConnectorItems(keysQuery.data ?? [], catalogQuery.data ?? []),
    [keysQuery.data, catalogQuery.data],
  );
  // Resolved from the live list rather than snapshotted at open time, so
  // revoking one of several connections updates the modal in place, and
  // revoking the last one unmounts it.
  const manageItem = items.added.find((item) => item.id === manageCardId);

  // A dead backend must not take the view away: report the failure as a
  // toast, render whatever data exists (or the normal empty states), and keep
  // the retry affordance inside the working view.
  const loadFailed = Boolean(keysQuery.error ?? catalogQuery.error);
  useEffect(() => {
    if (!loadFailed) return;
    toast.error("Could not load plugins", {
      id: "assistant-plugins-load-failed",
      description:
        "The assistant backend did not respond. Showing what has already loaded — retry below.",
    });
  }, [loadFailed]);

  if (keysQuery.isLoading || catalogQuery.isLoading) return <LoadingGrid />;

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
      {loadFailed ? (
        <div className="mb-4 flex items-center justify-between gap-3 rounded-xl border border-dashed border-border px-4 py-2.5">
          <p className="text-[12px] text-muted-foreground">
            Some plugins may be missing until the catalog loads.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              void keysQuery.refetch();
              void catalogQuery.refetch();
            }}
          >
            <RefreshCw />
            Retry
          </Button>
        </div>
      ) : null}
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
                    onManage={setManageCardId}
                    onConnect={setConnectSlug}
                  />
                ))}
              </CardGrid>
            </div>
          ) : (
            <p className="mb-7 rounded-xl border border-dashed border-border px-4 py-6 text-center text-[12px] text-muted-foreground">
              {loadFailed
                ? "Connected services could not be loaded right now."
                : "No connected services yet. Connect one below and your assistant can call it through the NyxID proxy."}
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
                onManage={setManageCardId}
                onConnect={setConnectSlug}
              />
            ))}
          </CardGrid>
        </>
      )}

      {manageItem && (
        <ManageConnectionModal
          keyIds={manageItem.manageKeyIds ?? []}
          serviceName={manageItem.name}
          iconSlug={manageItem.iconSlug}
          onClose={() => setManageCardId(null)}
        />
      )}

      {/* The same add-service dialog the Studio /keys page uses — handles every
          connect flavor (API-key paste, device-code) and invalidates the keys
          query on success, so the card moves to Added. OAuth opts into the
          managed popup on the same `cc` contract as the assistant's connect
          card, so the grid stays on screen while the user authorizes instead
          of the tab navigating away to the provider. */}
      {connectSlug !== null && (
        <AddKeyDialog
          open
          prefillSlug={connectSlug}
          launch="popup"
          flow="cc"
          onPopupViewResult={(keyId) => {
            const connecting = items.available.find(
              (candidate) => candidate.connectSlug === connectSlug,
            );
            setConnectSlug(null);
            // Deferred a tick so the add dialog unmounts before the manage
            // modal mounts; two overlapping Radix dialogs fight over focus.
            window.setTimeout(() => {
              setPopupResult({
                keyId,
                name: connecting?.name ?? connectSlug,
                iconSlug: connecting?.iconSlug ?? connectSlug,
              });
            }, 0);
            return true;
          }}
          onOpenChange={(next) => {
            if (!next) setConnectSlug(null);
          }}
        />
      )}

      {popupResult && (
        <ManageConnectionModal
          keyIds={[popupResult.keyId]}
          serviceName={popupResult.name}
          iconSlug={popupResult.iconSlug}
          onClose={() => setPopupResult(null)}
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
        actionLabel={item.added ? "Manage" : "Install"}
        // Skill management isn't built yet, so an installed skill's card is
        // inert rather than offering an action that does nothing.
        onActivate={
          item.added ? undefined : () => setAddedIds(addSkill(item.id))
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
