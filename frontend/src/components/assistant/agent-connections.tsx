import { describeChanges, changeList } from "@/lib/grouped-form-changes";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import {
  GroupedList,
  GroupedRow,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import { ArrowDown, ArrowUp, ChevronRight } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { useKeys } from "@/hooks/use-keys";
import {
  useServiceGroupOrder,
  type ServiceGroupOrder,
} from "@/hooks/use-service-group-order";
import {
  groupServiceConnections,
  type ServiceConnectionGroup,
} from "@/lib/service-groups";
import {
  connectionAvailable,
  connectionOwnerLabel,
} from "@/lib/assistant/connection-context";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import type { KeyInfo } from "@/types/keys";

export function AgentConnections({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  const keys = useKeys();
  const inventory = keys.data ?? [];
  const connections =
    agent.kind === "nyxbot"
      ? inventory
      : inventory.filter(
          (key) =>
            agent.services.includes(key.id) ||
            agent.services.includes(key.slug),
        );
  return (
    <section aria-label="Connections" className="space-y-3">
      <GroupedList
        title="Connections"
        footer={
          <>
            <p>
              Accounts grouped by service, so you can see which source this
              agent can use. A connection chooses an account. Folder, database
              and other resource restrictions depend on the service’s operation
              limits.
            </p>
            {keys.isError ? (
              <p role="alert" className="text-destructive">
                Could not load connections.
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow search={{ id: "manage-connections", title: "Manage connected accounts", breadcrumb: "Access › Connections" }} asChild interactive>
          <Link to="/keys">
            <span className="flex-1">Manage connected accounts</span>
            <ChevronRight aria-hidden className="size-[1.125em]" />
          </Link>
        </GroupedRow>
        {keys.isPending ? <GroupedRow title="Loading connections…" /> : null}
        {keys.isError ? (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void keys.refetch()}>
              Retry connections
            </button>
          </GroupedRow>
        ) : null}
        {!keys.isPending && !keys.isError && !connections.length ? (
          <GroupedRow
            title={
              agent.kind === "nyxbot"
                ? "Connect a service to choose the accounts NyxBot uses."
                : "Grant services in Access to make their connections available here."
            }
          />
        ) : null}
      </GroupedList>
      {agent.kind === "nyxbot" ? (
        <ConnectionPreferences
          inventory={inventory}
          disabled={
            agent.status === "destroyed" || agent.can_maintain === false
          }
        />
      ) : (
        <ConnectionGroups groups={groupServiceConnections(connections)} />
      )}
    </section>
  );
}

function ConnectionPreferences({
  inventory,
  disabled,
}: {
  readonly inventory: readonly KeyInfo[];
  readonly disabled: boolean;
}) {
  const order = useServiceGroupOrder(inventory);
  const groups = groupServiceConnections(order.inventory).map((group) => {
    const saved = order.savedOrder(group.id);
    return {
      ...group,
      connections: [
        ...saved.flatMap((id) =>
          group.connections.filter((key) => key.id === id),
        ),
        ...group.connections.filter((key) => !saved.includes(key.id)),
      ],
    };
  });
  return (
    <>
      <GroupedList
        title="Connection order"
        footer={
          <>
            <p>
              Your saved connection order applies across your agents. Eligible
              personal accounts are used before organization accounts.
            </p>
            {order.readError ? (
              <p role="alert" className="text-destructive">
                Could not load saved connection order.
              </p>
            ) : null}
            {order.unavailable ? (
              <p role="status">
                Saving connection order requires a server update.
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow title="Personal accounts first" />
        {order.readError ? (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void order.retryRead()}>
              Retry saved order
            </button>
          </GroupedRow>
        ) : null}
      </GroupedList>
      <ConnectionGroups groups={groups} order={order} disabled={disabled} />
    </>
  );
}

function ConnectionGroups({
  groups,
  order,
  disabled = false,
}: {
  readonly groups: readonly ServiceConnectionGroup[];
  readonly order?: ServiceGroupOrder;
  readonly disabled?: boolean;
}) {
  return (
    <div className="space-y-6">
      {groups.map((group) => {
        const editing = order?.groupId === group.id;
        const rows = editing ? order.connections : group.connections;
        return (
          <GroupedList key={group.id} title={group.name} info="Accounts connected to this service. Choose which account this agent should try first.">
            <div className="space-y-0">
              <ol aria-label={`${group.name} connections`}>
                {rows.map((key, index) => (
                  <GroupedRow search={{ id: `connection-${key.id}`, title: key.label, breadcrumb: `Access › Connections › ${group.name}`, keywords: `${key.slug} ${connectionOwnerLabel(key)} ${connectionAvailable(key) ? "Available" : "Unavailable"}` }} asChild key={key.id}>
                    <li>
                      <span className="min-w-0 flex-1 break-words">
                        <span className="block font-medium">{key.label}</span>
                        <span className="type-secondary text-text-tertiary">
                          {connectionOwnerLabel(key)} · {key.slug}
                          {connectionAvailable(key) ? "" : " · Unavailable"}
                        </span>
                      </span>
                      {editing && (
                        <>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={`Move ${key.label} up`}
                            disabled={
                              disabled ||
                              order.busy ||
                              order.blocked ||
                              index === 0
                            }
                            onClick={() => {
                              const ids = rows.map((row) => row.id);
                              [ids[index - 1], ids[index]] = [
                                ids[index]!,
                                ids[index - 1]!,
                              ];
                              order.update(ids);
                            }}
                          >
                            <ArrowUp aria-hidden className="size-3.5" />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={`Move ${key.label} down`}
                            disabled={
                              disabled ||
                              order.busy ||
                              order.blocked ||
                              index === rows.length - 1
                            }
                            onClick={() => {
                              const ids = rows.map((row) => row.id);
                              [ids[index], ids[index + 1]] = [
                                ids[index + 1]!,
                                ids[index]!,
                              ];
                              order.update(ids);
                            }}
                          >
                            <ArrowDown aria-hidden className="size-3.5" />
                          </Button>
                        </>
                      )}
                    </li>
                  </GroupedRow>
                ))}
              </ol>
              {order &&
                (editing ? (
                  <ConnectionOrderForm order={order} disabled={disabled} />
                ) : (
                  group.id.startsWith("catalog:") &&
                  rows.length > 1 && (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={disabled || Boolean(order.reason)}
                        title={order.reason}
                        onClick={() => order.start(group)}
                      >
                        Edit connection order
                      </button>
                    </GroupedRow>
                  )
                ))}
            </div>
          </GroupedList>
        );
      })}
    </div>
  );
}

function ConnectionOrderForm({
  order,
  disabled,
}: {
  readonly order: ServiceGroupOrder;
  readonly disabled: boolean;
}) {
  return (
    <form onSubmit={order.save} className="space-y-0">
      <GroupedFooter>
        {order.message && (
          <p role="status" className="text-13 text-text-tertiary">
            {order.message}
          </p>
        )}
        {order.validationError && (
          <p role="alert" className="text-13 text-destructive">
            {order.validationError}
          </p>
        )}
        {order.failure === "conflict" && (
          <p role="alert">
            Connection order changed elsewhere. Reload it before saving.
          </p>
        )}
        {order.failure === "stale" && (
          <p role="alert">Some connections are no longer available.</p>
        )}
        {order.failure === "network" && (
          <p role="alert">
            Could not save connection order. Your edits are kept.
          </p>
        )}
        {order.failure === "capacity" && (
          <p role="alert">
            {order.capacityMessage || "Connection order storage is full."}
          </p>
        )}
      </GroupedFooter>
      {order.failure === "conflict" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy}
            onClick={() => void order.recover(false)}
          >
            Reload order
          </button>
        </GroupedRow>
      )}
      {order.failure === "stale" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy}
            onClick={() => void order.refreshStale()}
          >
            Refresh connections
          </button>
        </GroupedRow>
      )}
      {order.failure === "network" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy || order.blocked}
            onClick={() => void order.retrySave()}
          >
            Retry save
          </button>
        </GroupedRow>
      )}
      {order.failure === "capacity" && (
        <GroupedRow asChild interactive>
          <Link to="/keys">Manage saved orders</Link>
        </GroupedRow>
      )}
      {!order.dirty && (
        <GroupedRow asChild interactive>
          <button type="button" disabled={order.busy} onClick={order.cancel}>
            Done
          </button>
        </GroupedRow>
      )}
      <GroupedRow asChild interactive>
        <button
          type="button"
          disabled={disabled || order.busy || order.blocked}
          onClick={order.reset}
        >
          Use default order
        </button>
      </GroupedRow>
      <GroupedFormActions
        changes={() => describeChanges({ order: order.groupId ? order.savedOrder(order.groupId) : [] }, { order: order.ordered }, [{ key: "order", label: "Account order", list: (value) => changeList(value).map((id) => { const key = order.inventory.find((key) => key.id === id); return key?.name || key?.label || "Unavailable account"; }) }])}
        label="connection order"
        dirty={order.dirty}
        pending={order.busy}
        disabled={disabled || order.blocked || Boolean(order.failure)}
        saveLabel="Save order"
        save={() => order.save()}
        discard={order.cancel}
      />
    </form>
  );
}
