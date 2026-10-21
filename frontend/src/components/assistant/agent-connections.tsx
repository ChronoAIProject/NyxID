import { ArrowDown, ArrowUp } from "lucide-react";
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
      <div className="space-y-1">
        <h3 className="text-13 font-semibold text-foreground">Connections</h3>
        <p className="text-12 text-muted-foreground">
          Accounts grouped by service, so you can see which source this agent
          can use.
        </p>
      </div>
      {keys.isPending && (
        <p role="status" className="text-12 text-muted-foreground">
          Loading connections…
        </p>
      )}
      {keys.isError && (
        <div role="alert" className="space-y-2 text-12">
          <p>Could not load connections.</p>
          <Button type="button" onClick={() => void keys.refetch()}>
            Retry connections
          </Button>
        </div>
      )}
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
      {!keys.isPending && !keys.isError && !connections.length && (
        <p className="rounded-lg border border-dashed border-border p-3 text-12 text-muted-foreground">
          {agent.kind === "nyxbot"
            ? "Connect a service to choose the accounts NyxBot uses."
            : "Grant services in Access to make their connections available here."}
        </p>
      )}
      <p className="text-11 text-muted-foreground">
        A connection chooses an account. Folder, database and other resource
        restrictions depend on the service's operation limits.
      </p>
      <Link
        to="/keys"
        className="inline-block text-12 underline underline-offset-4"
      >
        Manage connected accounts
      </Link>
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
      <p className="text-12 text-muted-foreground">
        Your saved connection order applies across your agents. Eligible
        personal accounts are used before organization accounts.
      </p>
      {order.readError && (
        <div role="alert" className="space-y-2 text-12">
          <p>Could not load saved connection order.</p>
          <Button type="button" onClick={() => void order.retryRead()}>
            Retry saved order
          </Button>
        </div>
      )}
      {order.unavailable && (
        <p role="status" className="text-12 text-muted-foreground">
          Saving connection order requires a server update.
        </p>
      )}
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
    <div className="space-y-3">
      {groups.map((group) => {
        const editing = order?.groupId === group.id;
        const rows = editing ? order.connections : group.connections;
        return (
          <details
            key={group.id}
            className="rounded-lg border border-border p-3"
            open={editing || undefined}
          >
            <summary className="cursor-pointer break-words text-12 font-medium">
              {group.name}
              <span className="ml-2 text-11 font-normal text-muted-foreground">
                {group.connections.length}{" "}
                {group.connections.length === 1 ? "account" : "accounts"}
              </span>
            </summary>
            <div className="mt-3 space-y-3">
              <ol
                aria-label={`${group.name} connections`}
                className="space-y-2"
              >
                {rows.map((key, index) => (
                  <li key={key.id} className="flex items-center gap-2 text-12">
                    <span className="min-w-0 flex-1 break-words">
                      <span className="block font-medium">{key.label}</span>
                      <span className="text-11 text-muted-foreground">
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
                ))}
              </ol>
              {order &&
                (editing ? (
                  <ConnectionOrderForm order={order} disabled={disabled} />
                ) : (
                  group.id.startsWith("catalog:") &&
                  rows.length > 1 && (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={disabled || Boolean(order.reason)}
                      title={order.reason}
                      onClick={() => order.start(group)}
                    >
                      Edit connection order
                    </Button>
                  )
                ))}
            </div>
          </details>
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
    <form onSubmit={order.save} className="space-y-3">
      {order.message && (
        <p role="status" className="text-12 text-muted-foreground">
          {order.message}
        </p>
      )}
      {order.validationError && (
        <p role="alert" className="text-12 text-destructive">
          {order.validationError}
        </p>
      )}
      {order.failure === "conflict" && (
        <div role="alert" className="space-y-2 text-12">
          <p>Connection order changed elsewhere. Reload it before saving.</p>
          <Button
            type="button"
            disabled={order.busy}
            onClick={() => void order.recover(false)}
          >
            Reload order
          </Button>
        </div>
      )}
      {order.failure === "stale" && (
        <div role="alert" className="space-y-2 text-12">
          <p>Some connections are no longer available.</p>
          <Button
            type="button"
            disabled={order.busy}
            onClick={() => void order.refreshStale()}
          >
            Refresh connections
          </Button>
        </div>
      )}
      {order.failure === "network" && (
        <div role="alert" className="space-y-2 text-12">
          <p>Could not save connection order. Your edits are kept.</p>
          <Button
            type="button"
            disabled={order.busy || order.blocked}
            onClick={() => void order.retrySave()}
          >
            Retry save
          </Button>
        </div>
      )}
      {order.failure === "capacity" && (
        <div role="alert" className="space-y-2 text-12">
          <p>{order.capacityMessage || "Connection order storage is full."}</p>
          <Link to="/keys" className="underline">
            Manage saved orders
          </Link>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={order.busy}
          onClick={order.cancel}
        >
          Cancel
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={disabled || order.busy || order.blocked}
          onClick={order.reset}
        >
          Use default order
        </Button>
        <Button
          type="submit"
          variant="primary"
          isLoading={order.busy}
          disabled={
            disabled ||
            order.busy ||
            order.blocked ||
            !order.dirty ||
            Boolean(order.failure)
          }
        >
          Save order
        </Button>
      </div>
    </form>
  );
}
