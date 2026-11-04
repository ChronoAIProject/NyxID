import { useEffect, useMemo, useState, type ReactNode } from "react";
import { ArrowRight, Plus, RefreshCw, Search } from "lucide-react";
import { toast } from "sonner";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedSelectTrigger,
} from "@/components/ui/grouped-list";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
  SegmentedControlContent,
} from "@/components/ui/segmented-control";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { AssistantLinkModal } from "./assistant-link-modals";
import { WorkspacePageHeader } from "./workspace-page-header";
import { ManageConnectionModal } from "./manage-connection-modal";
import { ServiceIcon } from "@/components/service-icon";
import { ServiceCardSummary } from "@/components/shared/service-card-summary";
import { useCreateAssistantConnectLink } from "@/hooks/use-connect-links";
import { useCatalog, useKeys } from "@/hooks/use-keys";
import {
  useAgentSkills,
  useAttachLatestSkill,
  useSetAgentSkills,
  useSkillSearch,
} from "@/hooks/use-agent-skills";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { agentTitle } from "@/lib/assistant/nyxbot-labels";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import {
  deriveConnectorItems,
  matchesPluginQuery,
  type ConnectorCardItem,
} from "@/lib/assistant/plugins";

interface PluginCardShape {
  readonly id: string;
  readonly name: string;
  readonly initial: string;
  readonly iconSlug?: string | null;
  readonly iconUrl?: string | null;
  readonly description: string;
  readonly meta: string;
  readonly added: boolean;
  readonly disabled?: boolean;
}

/** The primary native control covers the card. Secondary controls are siblings
 * above it, so they never nest interactive elements or trigger the primary. */
function PluginCard({
  item,
  status,
  actionLabel,
  onActivate,
  href,
  skillAgentId,
  secondary,
  add = false,
  disabled = false,
}: {
  readonly item: PluginCardShape;
  readonly status: string;
  readonly actionLabel: string;
  readonly onActivate?: () => void;
  readonly href?: { slug?: string };
  readonly skillAgentId?: string;
  readonly secondary?: ReactNode;
  readonly add?: boolean;
  readonly disabled?: boolean;
}) {
  const controlClass =
    "absolute inset-0 rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed";
  return (
    <article
      data-plugin-card=""
      className="assistant-plugin-card relative flex flex-col gap-3 rounded-xl border border-border/50 bg-card p-4 text-left transition-colors has-[button:not(:disabled):hover]:bg-overlay has-[a:hover]:bg-overlay"
    >
      {skillAgentId ? (
        <Link
          to="/assistant"
          search={{ agent: skillAgentId, details: "skills" }}
          className={controlClass}
          aria-label={`${actionLabel} ${item.name}`}
        />
      ) : href ? (
        <Link
          to="/keys"
          search={{ tab: "services", action: "add-service", ...href }}
          className={controlClass}
          aria-label={`${actionLabel} ${item.name}`}
        />
      ) : onActivate ? (
        <button
          type="button"
          className={controlClass}
          aria-label={add ? item.name : `${actionLabel} ${item.name}`}
          disabled={disabled}
          onClick={onActivate}
        />
      ) : null}
      <ServiceCardSummary
        description={item.description}
        name={item.name}
        subtitleTitle={status}
        className="pointer-events-none"
        icon={
          add ? <Plus aria-hidden className="size-[1.5em]" /> : item.iconSlug || item.iconUrl ? (
            <ServiceIcon
              slug={item.iconSlug}
              iconUrl={item.iconUrl}
              size="md"
            />
          ) : (
            item.initial
          )
        }
        subtitle={
          <>
            {item.added && !item.disabled ? (
              <span
                aria-hidden
                className="mr-1.5 inline-block size-[0.5em] rounded-full bg-success align-middle"
              />
            ) : null}
            {status}
          </>
        }
      />
      <div className="pointer-events-none mt-auto flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-1 text-13">
        <div className="pointer-events-auto relative flex min-w-0 items-center gap-2">
          {secondary}
        </div>
        <span
          aria-hidden
          className="flex items-center gap-1 type-row-title"
        >
          {actionLabel}
          {onActivate || href ? <ArrowRight className="size-[1em]" /> : null}
        </span>
      </div>
    </article>
  );
}

function SectionHeading({
  children,
  actions,
}: {
  readonly children: ReactNode;
  readonly actions?: ReactNode;
}) {
  return (
    <div className="mb-3 flex flex-wrap items-center justify-between gap-2 px-1">
      <h2 className="type-card-title">{children}</h2>
      {actions}
    </div>
  );
}
function CardGrid({ children }: { readonly children: ReactNode }) {
  return <div className="assistant-plugin-grid shell-card-grid grid">{children}</div>;
}
function LoadingGrid() {
  return (
    <div role="status" aria-label="Loading the plugin catalog">
      <CardGrid>
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="assistant-plugin-card rounded-xl" />
        ))}
      </CardGrid>
    </div>
  );
}
function ConnectorCard({
  item,
  onManage,
  onConnect,
  busy,
}: {
  readonly item: ConnectorCardItem;
  readonly onManage: (id: string) => void;
  readonly onConnect: (slug: string, personal: boolean) => void;
  readonly busy: boolean;
}) {
  const count = item.manageKeyIds?.length ?? 0;
  return (
    <PluginCard
      item={item}
      status={
        item.added
          ? `${item.disabled ? "Disabled" : "Connected"} · ${count} ${count === 1 ? "connection" : "connections"}`
          : item.meta
      }
      actionLabel={
        item.added
          ? "Manage"
          : item.advancedSetupSlug
            ? "Advanced setup"
            : "Connect"
      }
      href={
        item.advancedSetupSlug ? { slug: item.advancedSetupSlug } : undefined
      }
      disabled={!item.added && busy}
      onActivate={
        item.added
          ? () => onManage(item.id)
          : item.connectSlug
            ? () => onConnect(item.connectSlug!, false)
            : undefined
      }
      secondary={
        item.connectOwnSlug ? (
          <button
            type="button"
            disabled={busy}
            className="min-h-8 type-row-title text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => onConnect(item.connectOwnSlug!, true)}
          >
            Connect your own
          </button>
        ) : undefined
      }
    />
  );
}
function ConnectorsTab({ query }: { readonly query: string }) {
  const keysQuery = useKeys();
  const catalogQuery = useCatalog();
  const [manageCardId, setManageCardId] = useState<string | null>(null);
  const [connectHref, setConnectHref] = useState<string | null>(null);
  const [modelPicker, setModelPicker] = useState(false);
  const createConnect = useCreateAssistantConnectLink();
  function openConnect(slug: string, personal: boolean) {
    if (createConnect.isPending) return;
    void createConnect
      .mutateAsync({ serviceSlug: slug, personal })
      .then((href) => {
        setModelPicker(false);
        setConnectHref(href);
      })
      .catch((cause: unknown) => {
        toast.error(
          cause instanceof Error
            ? cause.message
            : "Could not create connection link",
        );
      });
  }
  const items = useMemo(
    () => deriveConnectorItems(keysQuery.data ?? [], catalogQuery.data ?? []),
    [keysQuery.data, catalogQuery.data],
  );
  const manageItem = items.added.find((item) => item.id === manageCardId);
  const loadFailed = Boolean(keysQuery.error ?? catalogQuery.error);
  useEffect(() => {
    if (loadFailed)
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
  const addModel: PluginCardShape = {
    id: "add-model",
    name: "Add a model connection",
    initial: "+",
    description:
      "Use your own provider account or a custom OpenAI-compatible endpoint.",
    meta: "Model providers",
    added: false,
  };
  const showModels = !searching || matchesPluginQuery({ ...addModel, category: "Models" }, query) || items.models.some((item) => matchesPluginQuery(item, query));
  function cards(rows: readonly ConnectorCardItem[]) {
    return rows.map((item) => (
      <ConnectorCard
        key={item.id}
        item={item}
        onManage={setManageCardId}
        onConnect={openConnect}
        busy={createConnect.isPending}
      />
    ));
  }
  return (
    <div className="space-y-7">
      {loadFailed ? (
        <GroupedList footer="Some plugins may be missing until the catalog loads.">
          <GroupedRow asChild interactive>
            <button
              type="button"
              onClick={() => {
                void keysQuery.refetch();
                void catalogQuery.refetch();
              }}
            >
              <RefreshCw aria-hidden className="size-[1.125em]" />
              Retry
            </button>
          </GroupedRow>
        </GroupedList>
      ) : null}
      {(added.length > 0 || !searching) && (
        <section aria-label="Connected">
          <SectionHeading>Connected</SectionHeading>
          {added.length ? (
            <CardGrid>{cards(added)}</CardGrid>
          ) : (
            <p className="px-4 py-6 text-14 text-text-tertiary">
              {loadFailed
                ? "Connected services could not be loaded right now."
                : "No connected services yet. Connect one below and your assistant can call it through the NyxID proxy."}
            </p>
          )}
        </section>
      )}
      {showModels && (
        <section aria-label="Models">
          <SectionHeading>Models</SectionHeading>
          <CardGrid>
            <PluginCard
              item={addModel}
              status="Model providers"
              actionLabel="Choose provider"
              add
              onActivate={() => setModelPicker(true)}
            />
          </CardGrid>
        </section>
      )}
      {available.length > 0 && (
        <section aria-label="Browse">
          <SectionHeading>Browse</SectionHeading>
          <CardGrid>{cards(available)}</CardGrid>
        </section>
      )}
      {searching &&
      !added.length &&
      !available.length &&
      !showModels ? (
        <p className="py-12 text-center text-14 text-text-tertiary">
          No plugins match this search.
        </p>
      ) : null}
      <GroupedList>
        <GroupedRow asChild interactive>
          <Link to="/keys" search={{ tab: "services", action: "add-service" }}>
            <span className="flex-1">Advanced setup</span>
            <ArrowRight
              aria-hidden
              className="size-[1.125em] text-text-tertiary"
            />
          </Link>
        </GroupedRow>
      </GroupedList>
      {manageItem && (
        <ManageConnectionModal
          keyIds={manageItem.manageKeyIds ?? []}
          serviceName={manageItem.name}
          iconSlug={manageItem.iconSlug}
          iconUrl={manageItem.iconUrl}
          onClose={() => setManageCardId(null)}
        />
      )}
      <Dialog open={modelPicker} onOpenChange={setModelPicker}>
        <DialogContent className="bg-background sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Add a model connection</DialogTitle>
            <DialogDescription>
              Use your own provider account or model endpoint.
            </DialogDescription>
          </DialogHeader>
          <GroupedList>
            {items.models.map((item) => (
              <GroupedRow asChild interactive key={item.id}>
                <button
                  type="button"
                  disabled={createConnect.isPending}
                  onClick={() => openConnect(item.connectSlug!, true)}
                >
                  <ServiceIcon slug={item.iconSlug} size="sm" />
                  <span className="flex-1">{item.name}</span>
                  <ArrowRight
                    aria-hidden
                    className="size-[1.125em] text-text-tertiary"
                  />
                </button>
              </GroupedRow>
            ))}
            <GroupedRow asChild interactive>
              <Link
                to="/keys"
                search={{ tab: "services", action: "add-service" }}
              >
                <ServiceIcon slug="custom" size="sm" />
                Custom OpenAI-compatible endpoint
                <ArrowRight
                  aria-hidden
                  className="ml-auto size-[1.125em] text-text-tertiary"
                />
              </Link>
            </GroupedRow>
          </GroupedList>
        </DialogContent>
      </Dialog>
      <AssistantLinkModal
        href={connectHref}
        onClose={() => setConnectHref(null)}
      />
    </div>
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
    <GroupedList>
      <GroupedRow
        title="Agent"
        accessory={
          <Select value={value} onValueChange={onChange}>
            <GroupedSelectTrigger aria-label="Agent for skills">
              <SelectValue placeholder="Choose an agent" />
            </GroupedSelectTrigger>
            <SelectContent>
              {agents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>
                  {agentTitle(agent)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
      />
    </GroupedList>
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
  const search = useSkillSearch(query.trim(), 1);
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
        status={item.added ? "Attached" : "Ornn catalog"}
        disabled={busy || full}
        actionLabel={item.added ? "View" : "Attach"}
        skillAgentId={item.added ? agentId : undefined}
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
        <p className="text-14 text-muted-foreground">
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
        <p key={index} role="alert" className="text-14 text-destructive">
          {error.message}
        </p>
      ))}
      {agent && attached.data ? (
        <GroupedList
          title={`Attached to ${agentTitle(agent)} · ${pins.length}/16`}
          footer={
            <>
              <p>
                Versions, tests and edits live in the agent&apos;s settings.
              </p>
              {full ? (
                <p>This agent already has 16 skills. Detach one first.</p>
              ) : null}
            </>
          }
        >
          {pins.length ? (
            pins.map((pin) => (
              <GroupedRow
                key={pin.skill_id}
                title={pin.name}
                description={pin.disabled ? "Disabled" : undefined}
                accessory={
                  <>
                    <span>{pin.version}</span>
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
                    >{`Detach ${pin.name}`}</Button>
                  </>
                }
              />
            ))
          ) : (
            <GroupedRow
              title="No skills attached"
              description="Search the catalog to attach one."
            />
          )}
        </GroupedList>
      ) : null}
      {!query.trim() && !items.length ? (
        <GroupedList>
          <GroupedRow
            title="Find a skill"
            description="Search by name or workflow to explore the Ornn catalog."
          />
        </GroupedList>
      ) : null}
      {query.trim() && items.length === 0 && search.data ? (
        <p className="py-14 text-center text-13 text-text-tertiary">
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
  const [tab, setTab] = useState("connector");
  const [query, setQuery] = useState("");
  return (
    <div className="shell-page-stack pb-4">
      <WorkspacePageHeader
        title="Plugins"
        description="Connect your services and model accounts. Add skills to teach your assistant new workflows."
      />
      <SegmentedControl
        value={tab}
        onValueChange={setTab}
        className="space-y-6"
      >
        <SegmentedControlList aria-label="Plugin type" className="max-w-80">
          <SegmentedControlTrigger value="connector">
            Connectors
          </SegmentedControlTrigger>
          <SegmentedControlTrigger value="skill">
            Skills
          </SegmentedControlTrigger>
        </SegmentedControlList>
        <GroupedList className="max-w-2xl">
          <GroupedRow leading={<Search aria-hidden />}>
            <Search
              aria-hidden
              className="size-[1.125em] shrink-0 text-text-tertiary"
            />
            <GroupedInput
              className="min-w-0 flex-1"
              aria-label="Search plugins"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search the catalog..."
            />
          </GroupedRow>
        </GroupedList>
        <SegmentedControlContent value="connector">
          <ConnectorsTab query={query} />
        </SegmentedControlContent>
        <SegmentedControlContent value="skill">
          <SkillsTab query={query} agentId={agentId} />
        </SegmentedControlContent>
      </SegmentedControl>
    </div>
  );
}
