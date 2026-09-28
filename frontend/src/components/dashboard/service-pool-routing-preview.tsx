import { useState } from "react";
import { ArrowDown, ArrowUp, GripVertical, RefreshCw } from "lucide-react";
import { useServicePools } from "@/hooks/use-pools";
import { useKeys } from "@/hooks/use-keys";
import { useUserServices } from "@/hooks/use-user-services";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { ServicePool } from "@/schemas/pools";
import {
  classifyConnection,
  moveItem,
  orderedIds,
  readPreferences,
  savePreferences,
  type PreviewPreferences,
  type RoutingCandidate,
} from "@/lib/service-routing-preview";
import {
  ConnectionCard,
  type RoutingPreviewProps,
} from "./service-routing-preview";

function PoolCard({
  pool,
  candidates,
  preference,
  onChange,
  renderConnection,
}: RoutingPreviewProps & {
  readonly pool: ServicePool;
  readonly candidates: ReadonlyMap<string, RoutingCandidate>;
  readonly preference: PreviewPreferences["pools"][string] | undefined;
  readonly onChange: (value: PreviewPreferences["pools"][string]) => void;
}) {
  const [dragged, setDragged] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const priority = preference?.priority ?? false;
  const ids = orderedIds(
    pool.members.map((member) => member.user_service_id),
    priority ? (preference?.order ?? []) : [],
  );
  const strategy = pool.strategy === "weighted" ? "Weighted" : "Round robin";
  const label = (id: string) =>
    candidates.get(id)?.key.label ?? "Missing service";

  function reorder(from: string, to: string) {
    if (
      !priority ||
      !pool.is_active ||
      !ids.includes(from) ||
      !ids.includes(to)
    )
      return;
    const next = moveItem(ids, from, to);
    onChange({ priority, order: next });
    setAnnouncement(
      `${label(from)} moved to position ${next.indexOf(from) + 1} in ${pool.name}.`,
    );
  }

  return (
    <section
      aria-label={pool.name}
      className="space-y-4 rounded-xl border bg-card/30 p-4 sm:p-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-semibold">{pool.name}</h3>
            <Badge variant="secondary">{pool.members.length} members</Badge>
            {!pool.is_active && <Badge variant="secondary">Disabled</Badge>}
          </div>
          <code className="text-xs text-muted-foreground">
            /proxy/s/{pool.slug}
          </code>
        </div>
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Selection
          <select
            aria-label={`Selection for ${pool.name}`}
            value={priority ? "priority" : "current"}
            onChange={(event) =>
              onChange({
                priority: event.target.value === "priority",
                order: preference?.order ?? ids,
              })
            }
            className="h-9 rounded-md border bg-background px-2 text-sm text-foreground"
          >
            <option value="current">{strategy} (current)</option>
            <option value="priority">Priority order (preview)</option>
          </select>
        </label>
      </div>
      {pool.description && (
        <p className="text-sm text-muted-foreground">{pool.description}</p>
      )}
      {priority && (
        <p className="text-xs text-muted-foreground">
          Drag members to set a preference. Working connections still need
          verification.
        </p>
      )}
      <span aria-live="polite" className="sr-only">
        {announcement}
      </span>
      <ol
        aria-label={`Members of ${pool.name}`}
        className="grid items-start gap-4 sm:grid-cols-2 xl:grid-cols-3"
      >
        {ids.map((id, index) => {
          const member = pool.members.find(
            (item) => item.user_service_id === id,
          )!;
          const candidate = candidates.get(id);
          const reorderable = priority && pool.is_active;
          return (
            <li
              key={id}
              className={cn(
                "min-w-0 space-y-2 rounded-lg transition-colors",
                over === id && dragged !== id && "ring-2 ring-primary/50",
                dragged === id && "opacity-50",
              )}
              onDragOver={(event) => {
                if (!reorderable || !dragged) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                setOver(id);
              }}
              onDrop={(event) => {
                if (!reorderable || !dragged) return;
                event.preventDefault();
                if (
                  event.dataTransfer.getData("application/x-nyxid-pool") ===
                  pool.id
                )
                  reorder(dragged, id);
                setDragged(null);
                setOver(null);
              }}
            >
              <div className="flex h-8 items-center justify-between gap-2 px-1 text-xs text-muted-foreground">
                <span>
                  {priority
                    ? `Preference ${index + 1}`
                    : pool.strategy === "weighted"
                      ? `Weight ${member.weight}`
                      : `Member ${index + 1}`}
                </span>
                <div className="flex items-center gap-1">
                  {!member.enabled && (
                    <Badge variant="secondary">Excluded</Badge>
                  )}
                  {priority && (
                    <>
                      <button
                        type="button"
                        draggable={reorderable}
                        disabled={!reorderable}
                        aria-label={`Drag ${label(id)} in ${pool.name}`}
                        title="Drag or use the arrow keys"
                        className="cursor-grab rounded p-1.5 hover:bg-muted focus-visible:outline focus-visible:outline-primary disabled:cursor-default disabled:opacity-40 active:cursor-grabbing"
                        onDragStart={(event) => {
                          event.dataTransfer.effectAllowed = "move";
                          event.dataTransfer.setData(
                            "application/x-nyxid-pool",
                            pool.id,
                          );
                          event.dataTransfer.setData("text/plain", id);
                          setDragged(id);
                        }}
                        onDragEnd={() => {
                          setDragged(null);
                          setOver(null);
                        }}
                        onKeyDown={(event) => {
                          if (
                            event.key !== "ArrowUp" &&
                            event.key !== "ArrowDown"
                          )
                            return;
                          event.preventDefault();
                          const target =
                            ids[index + (event.key === "ArrowUp" ? -1 : 1)];
                          if (target) reorder(id, target);
                        }}
                      >
                        <GripVertical className="size-4" />
                      </button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="size-7"
                        disabled={!reorderable || index === 0}
                        aria-label={`Move ${label(id)} up in ${pool.name}`}
                        onClick={() => reorder(id, ids[index - 1]!)}
                      >
                        <ArrowUp className="size-3" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="size-7"
                        disabled={!reorderable || index === ids.length - 1}
                        aria-label={`Move ${label(id)} down in ${pool.name}`}
                        onClick={() => reorder(id, ids[index + 1]!)}
                      >
                        <ArrowDown className="size-3" />
                      </Button>
                    </>
                  )}
                </div>
              </div>
              {candidate ? (
                <ConnectionCard
                  candidate={candidate}
                  renderConnection={renderConnection}
                />
              ) : (
                <div className="rounded-xl border border-dashed p-4 text-sm text-muted-foreground">
                  Service unavailable
                  <code className="mt-2 block break-all text-xs">{id}</code>
                </div>
              )}
            </li>
          );
        })}
      </ol>
      {ids.length === 0 && (
        <p className="py-6 text-center text-sm text-muted-foreground">
          No members in this pool.
        </p>
      )}
    </section>
  );
}

function PoolRoutingContent({
  userId,
  renderConnection,
}: RoutingPreviewProps & { readonly userId: string | undefined }) {
  const pools = useServicePools();
  const keys = useKeys();
  const services = useUserServices();
  const [preferences, setPreferences] = useState(() => readPreferences(userId));
  const [mountedAt] = useState(Date.now);
  const candidates = new Map(
    (keys.data ?? []).map((key) => [
      key.id,
      classifyConnection(
        key,
        services.data ?? [],
        keys.dataUpdatedAt || mountedAt,
      ),
    ]),
  );

  if (pools.isLoading || keys.isLoading)
    return <Skeleton className="h-64 w-full" />;
  if (pools.error || keys.error)
    return (
      <div className="rounded-xl border p-6 text-sm">
        Pool data could not be loaded.{" "}
        <Button
          variant="outline"
          onClick={() => {
            void pools.refetch();
            void keys.refetch();
          }}
        >
          Retry
        </Button>
      </div>
    );
  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span>Your pools · Local preview · Changes stay in this browser</span>
        <Button
          size="icon"
          variant="ghost"
          aria-label="Refresh pool metadata"
          onClick={() => {
            void pools.refetch();
            void keys.refetch();
            void services.refetch();
          }}
        >
          <RefreshCw className="size-4" />
        </Button>
      </div>
      {(pools.data ?? []).map((pool) => (
        <PoolCard
          key={pool.id}
          pool={pool}
          candidates={candidates}
          preference={preferences.pools[pool.id]}
          renderConnection={renderConnection}
          onChange={(value) => {
            const stored = readPreferences(userId);
            const next = {
              ...stored,
              pools: { ...stored.pools, [pool.id]: value },
            };
            setPreferences(next);
            savePreferences(userId, next);
          }}
        />
      ))}
      {pools.data?.length === 0 && (
        <p className="py-12 text-center text-sm text-muted-foreground">
          No service pools in your account.
        </p>
      )}
    </div>
  );
}

export default function ServicePoolRoutingPreview(props: RoutingPreviewProps) {
  const userId = useAuthStore((state) => state.user?.id);
  return (
    <PoolRoutingContent
      key={userId ?? "anonymous"}
      {...props}
      userId={userId}
    />
  );
}
