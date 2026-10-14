import { useEffect, useMemo, useState } from "react";
import { ArrowRight, RefreshCw, Search } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ManageConnectionModal } from "@/components/assistant/manage-connection-modal";
import { ServiceIcon } from "@/components/service-icon";
import { useCatalog, useKeys } from "@/hooks/use-keys";
import {
  useAgentSkills,
  useAttachLatestSkill,
  useSetAgentSkills,
  useSkillSearch,
} from "@/hooks/use-agent-skills";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { agentTitle } from "@/lib/assistant/nyxbot-labels";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import {
  deriveConnectorItems,
  matchesPluginQuery,
  type ConnectorCardItem,
  type PluginKind,
} from "@/lib/assistant/plugins";

interface PluginCardShape {
  readonly id: string;
  readonly name: string;
  readonly initial: string;
  readonly iconSlug?: string | null;
  readonly iconUrl?: string | null;
  readonly category: string;
  readonly description: string;
  readonly meta: string;
  readonly added: boolean;
  /** Added connectors only: every connection behind the card is disabled. */
  readonly disabled?: boolean;
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
    <div className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-md bg-muted text-13 font-semibold text-muted-foreground">
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
  readonly addedBadge: "Connected" | "Installed" | "Attached";
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
          {item.iconSlug || item.iconUrl ? (
            <ServiceIcon slug={item.iconSlug} iconUrl={item.iconUrl} size="md" />
          ) : (
            item.initial
          )}
        </PluginTile>
        <div className="min-w-0">
          <p className="truncate text-13 font-medium text-foreground">
            {item.name}
          </p>
          <p className="text-10 uppercase tracking-[0.5px] text-text-tertiary">
            {item.category}
          </p>
        </div>
      </div>
      {/* The spacer absorbs the card's slack so the action row stays pinned to
          the bottom; the clamp lives on the text itself, or `flex-1` would
          stretch the box past three lines and clip a fourth mid-glyph. */}
      <div className="min-h-0 flex-1">
        <p className="break-words text-12 leading-[17px] text-muted-foreground line-clamp-3">
          {item.description}
        </p>
      </div>
      <div className="flex shrink-0 items-center justify-between gap-2">
        {item.added ? (
          <span className="flex min-w-0 items-center gap-2">
            {/* A disabled connection is still added — it just can't be used.
                Showing the green Connected badge would claim the assistant has
                a working way in, so the neutral Disabled badge replaces it
                rather than sitting beside it. */}
            {item.disabled ? (
              <Badge variant="secondary">Disabled</Badge>
            ) : (
              <Badge variant="success">{addedBadge}</Badge>
            )}
            {addedMeta && (
              <span className="truncate font-mono text-10 text-text-tertiary">
                {addedMeta}
              </span>
            )}
          </span>
        ) : (
          <span className="font-mono text-10 text-text-tertiary">
            {item.meta}
          </span>
        )}
        <span
          aria-hidden
          className={`flex shrink-0 items-center gap-1 text-12 transition-colors ${
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

function SectionHeading({ children }: { readonly children: React.ReactNode }) {
  return (
    <p className="mb-2.5 text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary">
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
    readonly iconUrl: string | null;
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
      <p className="py-14 text-center text-11 text-text-tertiary">
        No plugins match this search.
      </p>
    );
  }

  return (
    <>
      {loadFailed ? (
        <div className="mb-4 flex items-center justify-between gap-3 rounded-xl border border-dashed border-border px-4 py-2.5">
          <p className="text-12 text-muted-foreground">
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
            <p className="mb-7 rounded-xl border border-dashed border-border px-4 py-6 text-center text-12 text-muted-foreground">
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
          iconUrl={manageItem.iconUrl}
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
                iconUrl: connecting?.iconUrl ?? null,
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
          iconUrl={popupResult.iconUrl}
          onClose={() => setPopupResult(null)}
        />
      )}
    </>
  );
}

/** The agent whose skills the tab manages: NyxBot unless the owner picks another. */
function SkillAgentPicker({
  agents,
  value,
  onChange,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly value: string;
  readonly onChange: (id: string) => void;
}) {
  return (
    <label className="flex max-w-[300px] flex-col gap-1 text-11 text-text-tertiary">
      Agent
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger aria-label="Agent for skills">
          <SelectValue placeholder="Choose an agent" />
        </SelectTrigger>
        <SelectContent>
          {agents.map((agent) => (
            <SelectItem key={agent.id} value={agent.id}>
              {agentTitle(agent)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}

function SkillsTab({
  query,
  agentId: initialAgentId,
}: {
  readonly query: string;
  readonly agentId?: string;
}) {
  const agents = useNyxBotAgents();
  // Destroyed agents and agents the owner cannot maintain keep their skills.
  const manageable = (agents.data?.agents ?? []).filter(
    (agent) => agent.status !== "destroyed" && agent.can_maintain !== false,
  );
  const [chosen, setChosen] = useState(initialAgentId ?? "");
  const agent =
    manageable.find((item) => item.id === chosen) ??
    manageable.find((item) => item.kind === "nyxbot") ??
    manageable[0];
  const agentId = agent?.id ?? "";
  const search = useSkillSearch(query.trim() || null, 1);
  const attached = useAgentSkills(agentId);
  const detach = useSetAgentSkills(agentId);
  const attach = useAttachLatestSkill(agentId);
  const pins = attached.data?.skills ?? [];
  const busy = !attached.data || detach.isPending || attach.isPending;
  const full = pins.length >= 16;
  const items: PluginCardShape[] = (search.data?.items ?? []).map((skill) => ({
    id: skill.id,
    name: skill.name,
    initial: skill.name.slice(0, 2).toUpperCase(),
    category: "Skill",
    description: skill.description,
    meta: "Ornn catalog",
    added: pins.some((pin) => pin.skill_id === skill.id),
  }));
  const errors = [
    agents.error,
    attached.error,
    search.error,
    attach.error,
    detach.error,
  ].filter((error): error is Error => Boolean(error));

  function card(item: PluginCardShape) {
    return (
      <PluginCard
        key={item.id}
        item={item}
        addedBadge="Attached"
        actionLabel={item.added ? "Attached" : "Attach"}
        onActivate={
          item.added
            ? undefined
            : () => {
                if (busy || !attached.data) return;
                if (full) {
                  toast.info(
                    "This agent already has 16 skills. Detach one first.",
                  );
                  return;
                }
                attach.mutate({ skillId: item.id, current: attached.data });
              }
        }
      />
    );
  }

  return (
    <div className="space-y-5">
      {agents.isPending ? (
        <Skeleton className="h-8 w-[300px]" />
      ) : manageable.length === 0 && !agents.error ? (
        <p className="text-12 text-muted-foreground">
          No agent you can maintain is available.
        </p>
      ) : agent ? (
        <SkillAgentPicker
          agents={manageable}
          value={agent.id}
          onChange={setChosen}
        />
      ) : null}
      {errors.map((error, index) => (
        <p key={index} role="alert" className="text-12 text-destructive">
          {error.message}
        </p>
      ))}
      {agent && attached.data ? (
        <section aria-label="Attached skills" className="space-y-2">
          <SectionHeading>
            {`Attached to ${agentTitle(agent)} · ${pins.length}/16`}
          </SectionHeading>
          {pins.length === 0 ? (
            <p className="text-12 text-muted-foreground">
              No skills attached. Search the catalog to attach one.
            </p>
          ) : (
            <ul className="space-y-2">
              {pins.map((pin) => (
                <li
                  key={pin.skill_id}
                  className="flex items-center justify-between gap-3 rounded-lg border border-border/50 px-3 py-2"
                >
                  <span className="min-w-0 text-12">
                    <span className="font-medium">{pin.name}</span>{" "}
                    <span className="font-mono text-text-tertiary">
                      {pin.version}
                    </span>
                    {pin.disabled ? (
                      <span className="text-text-tertiary"> · Disabled</span>
                    ) : null}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() =>
                      detach.mutate({
                        expected_revision: attached.data.revision,
                        skills: pins.filter(
                          (item) => item.skill_id !== pin.skill_id,
                        ),
                      })
                    }
                  >
                    {`Detach ${pin.name}`}
                  </Button>
                </li>
              ))}
            </ul>
          )}
          <p className="text-11 text-text-tertiary">
            Versions, tests and edits live in the agent&apos;s settings.
          </p>
        </section>
      ) : null}
      {query.trim() && items.length === 0 && search.data ? (
        <p className="py-14 text-center text-11 text-text-tertiary">
          No plugins match this search.
        </p>
      ) : null}
      {items.length > 0 && (
        <div>
          <SectionHeading>Ornn catalog</SectionHeading>
          <CardGrid>{items.map(card)}</CardGrid>
        </div>
      )}
    </div>
  );
}

export function PluginsView({ agentId }: { readonly agentId?: string } = {}) {
  const [tab, setTab] = useState<PluginKind>("connector");
  const [query, setQuery] = useState("");

  return (
    <div className="h-full min-h-0 overflow-y-auto overscroll-contain">
      <div className="px-5 pt-6 sm:px-8">
        <h1 className="text-22 font-bold tracking-[-0.03em] sm:text-28">
          Plugins
        </h1>
        <p className="mt-1 max-w-2xl text-12 text-muted-foreground">
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
              className="min-w-0 flex-1 bg-transparent text-12 text-foreground outline-none placeholder:text-text-tertiary"
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
                className={`border-b-2 px-3 text-12 transition-colors ${
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
          <SkillsTab query={query} agentId={agentId} />
        )}
      </div>
    </div>
  );
}
