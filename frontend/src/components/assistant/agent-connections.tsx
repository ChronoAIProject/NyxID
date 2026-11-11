import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
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
  useUiTranslation();
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
    <section aria-label={translateUi("Connections")} className="space-y-3">
      <GroupedList
        title={translateUi("Connections")}
        footer={
          <>
            <p>{translateUi("Accounts grouped by service, so you can see which source this agent can use. A connection chooses an account. Folder, database and other resource restrictions depend on the service’s operation limits.")}</p>
            {keys.isError ? (
              <p role="alert" className="text-destructive">{translateUi("Could not load connections.")}</p>
            ) : null}
          </>
        }
      >
        <GroupedRow search={{ id: "manage-connections", title: "Manage connected accounts", breadcrumb: "Access › Connections" }} asChild interactive>
          <Link to="/keys">
            <span className="flex-1">{translateUi("Manage connected accounts")}</span>
            <ChevronRight aria-hidden className="size-[1.125em]" />
          </Link>
        </GroupedRow>
        {keys.isPending ? <GroupedRow title={translateUi("Loading connections…")} /> : null}
        {keys.isError ? (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void keys.refetch()}>{translateUi("Retry connections")}</button>
          </GroupedRow>
        ) : null}
        {!keys.isPending && !keys.isError && !connections.length ? (
          <GroupedRow
            title={
              agent.kind === "nyxbot"
                ? translateUi("Connect a service to choose the accounts NyxBot uses.")
                : translateUi("Grant services in Access to make their connections available here.")
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
  useUiTranslation();
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
        title={translateUi("Connection order")}
        footer={
          <>
            <p>{translateUi("Your saved connection order applies across your agents. Eligible personal accounts are used before organization accounts.")}</p>
            {order.readError ? (
              <p role="alert" className="text-destructive">{translateUi("Could not load saved connection order.")}</p>
            ) : null}
            {order.unavailable ? (
              <p role="status">{translateUi("Saving connection order requires a server update.")}</p>
            ) : null}
          </>
        }
      >
        <GroupedRow title={translateUi("Personal accounts first")} />
        {order.readError ? (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void order.retryRead()}>{translateUi("Retry saved order")}</button>
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
  useUiTranslation();
  return (
    <div className="space-y-6">
      {groups.map((group) => {
        const editing = order?.groupId === group.id;
        const rows = editing ? order.connections : group.connections;
        return (
          <GroupedList key={group.id} title={group.name} info="Accounts connected to this service. Choose which account this agent should try first.">
            <div className="space-y-0">
              <ol aria-label={translateUi("{{value1}} connections", { value1: group.name })}>
                {rows.map((key, index) => (
                  <GroupedRow search={{ id: `connection-${key.id}`, title: key.label, breadcrumb: `Access › Connections › ${group.name}`, keywords: `${key.slug} ${connectionOwnerLabel(key)} ${connectionAvailable(key) ? "Available" : "Unavailable"}` }} asChild key={key.id}>
                    <li>
                      <span className="min-w-0 flex-1 break-words">
                        <span className="block font-medium">{key.label}</span>
                        <span className="type-secondary text-text-tertiary">
                          {connectionOwnerLabel(key)} · {key.slug}
                          {connectionAvailable(key) ? "" : translateUi(" · Unavailable")}
                        </span>
                      </span>
                      {editing && (
                        <>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={translateUi("Move {{value1}} up", { value1: key.label })}
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
                            aria-label={translateUi("Move {{value1}} down", { value1: key.label })}
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
                      >{translateUi("Edit connection order")}</button>
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
  useUiTranslation();
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
          <p role="alert">{translateUi("Connection order changed elsewhere. Reload it before saving.")}</p>
        )}
        {order.failure === "stale" && (
          <p role="alert">{translateUi("Some connections are no longer available.")}</p>
        )}
        {order.failure === "network" && (
          <p role="alert">{translateUi("Could not save connection order. Your edits are kept.")}</p>
        )}
        {order.failure === "capacity" && (
          <p role="alert">
            {order.capacityMessage || translateUi("Connection order storage is full.")}
          </p>
        )}
      </GroupedFooter>
      {order.failure === "conflict" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy}
            onClick={() => void order.recover(false)}
          >{translateUi("Reload order")}</button>
        </GroupedRow>
      )}
      {order.failure === "stale" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy}
            onClick={() => void order.refreshStale()}
          >{translateUi("Refresh connections")}</button>
        </GroupedRow>
      )}
      {order.failure === "network" && (
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={order.busy || order.blocked}
            onClick={() => void order.retrySave()}
          >{translateUi("Retry save")}</button>
        </GroupedRow>
      )}
      {order.failure === "capacity" && (
        <GroupedRow asChild interactive>
          <Link to="/keys">{translateUi("Manage saved orders")}</Link>
        </GroupedRow>
      )}
      {!order.dirty && (
        <GroupedRow asChild interactive>
          <button type="button" disabled={order.busy} onClick={order.cancel}>{translateUi("Done")}</button>
        </GroupedRow>
      )}
      <GroupedRow asChild interactive>
        <button
          type="button"
          disabled={disabled || order.busy || order.blocked}
          onClick={order.reset}
        >{translateUi("Use default order")}</button>
      </GroupedRow>
      <GroupedFormActions
        changes={() => describeChanges({ order: order.groupId ? order.savedOrder(order.groupId) : [] }, { order: order.ordered }, [{ key: "order", label: "Account order", list: (value) => changeList(value).map((id) => { const key = order.inventory.find((key) => key.id === id); return key?.name || key?.label || "Unavailable account"; }) }])}
        label={translateUi("connection order")}
        dirty={order.dirty}
        pending={order.busy}
        disabled={disabled || order.blocked || Boolean(order.failure)}
        saveLabel={translateUi("Save order")}
        save={() => order.save()}
        discard={order.cancel}
      />
    </form>
  );
}
