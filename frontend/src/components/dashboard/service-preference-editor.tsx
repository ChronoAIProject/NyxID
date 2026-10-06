import { useAuthStore } from "@/stores/auth-store";
import { useEffect, useRef, useState } from "react";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { zodResolver } from "@hookform/resolvers/zod";
import { GripVertical, Info } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useAppForm } from "@/components/ui/form";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ApiError } from "@/lib/api-client";
import { useSaveServicePreference } from "@/hooks/use-service-preference";
import {
  MAX_ORDERED_SERVICES,
  servicePreferenceRequestSchema,
  type ServicePreference,
  type ServicePreferenceRequest,
} from "@/schemas/service-preference";
import type { KeyInfo } from "@/types/keys";
import type { ViewMode } from "@/components/shared/view-toggle";

import { ServiceIcon } from "@/components/service-icon";
import { ServiceOwnerAvatar } from "./service-owner-avatar";
import { connectionSource, connectionSourceLabel } from "@/lib/service-view";

const DIVIDER = "__unranked__";
function initialItems(
  preference: ServicePreference,
  inventory: readonly KeyInfo[],
) {
  const visible = new Set(inventory.map((item) => item.id));
  const ranked = preference.ordered.filter((id) => visible.has(id));
  const rankedSet = new Set(ranked);
  return [
    ...ranked,
    DIVIDER,
    ...inventory
      .filter((item) => !rankedSet.has(item.id))
      .map((item) => item.id),
  ];
}
function orderedIds(items: readonly string[]) {
  return items.slice(0, items.indexOf(DIVIDER));
}

function SortableItem({
  id,
  item,
  rank,
  disabled,
  viewMode,
  onToggle,
}: {
  readonly id: string;
  readonly item?: KeyInfo;
  readonly rank?: number;
  readonly disabled: boolean;
  readonly viewMode: ViewMode;
  readonly onToggle: () => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
    isOver,
  } = useSortable({ id, disabled });
  const label = item?.label ?? "Unranked divider";
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      data-preference-item={id}
      data-drop-target={isOver || undefined}
      className={`flex min-w-0 items-center gap-3 rounded-xl border p-3 ${viewMode === "grid" && item ? "min-h-24" : ""} ${isDragging ? "opacity-35" : ""} ${isOver ? "ring-2 ring-accent" : ""} ${item ? "bg-card" : "border-dashed bg-overlay"}`}
    >
      <Button
        type="button"
        variant="ghost"
        size="icon"
        ref={setActivatorNodeRef}
        {...attributes}
        {...listeners}
        className="shrink-0 touch-none cursor-grab active:cursor-grabbing"
        aria-label={`Drag ${label}`}
        disabled={disabled}
      >
        <GripVertical className="h-4 w-4" />
      </Button>
      {item && (
        <ServiceIcon
          slug={item.catalog_service_slug ?? item.slug}
          iconUrl={item.icon_url}
          size="sm"
        />
      )}
      <div className="min-w-0 flex-1">
        <p className="break-words text-12 font-medium">{label}</p>
        {item ? (
          <>
            <p className="mt-1 break-words text-11 text-muted-foreground [overflow-wrap:anywhere]">
              <code>{item.slug}</code> ·{" "}
              {item.catalog_service_name ?? "Custom service"}
            </p>
            <div className="mt-1 flex flex-wrap items-center gap-1.5 text-11 text-muted-foreground">
              <ServiceOwnerAvatar
                type={connectionSource(item)}
                name={connectionSourceLabel(item)}
                avatarUrl={
                  item.credential_source?.type === "org"
                    ? item.credential_source.avatar_url
                    : undefined
                }
              />
              <Badge
                variant="secondary"
                className="max-w-full whitespace-normal break-words"
              >
                {item.credential_source?.type === "org" ? "Org: " : ""}
                {connectionSourceLabel(item)}
              </Badge>
              {item.auto_connected && (
                <Badge variant="secondary">Auto-connected</Badge>
              )}
              {!item.is_active && <Badge variant="secondary">Disabled</Badge>}
            </div>
          </>
        ) : (
          <p className="mt-1 text-11 text-muted-foreground">
            Services below this divider have no preference rank
          </p>
        )}
      </div>
      {item && (
        <div className="flex shrink-0 flex-col items-end gap-2 sm:flex-row sm:items-center">
          <Badge
            variant={rank ? "accent" : "secondary"}
            aria-label={rank ? `Discovery preference ${rank}` : undefined}
          >
            {rank ? `Discovery #${rank}` : "Unranked"}
          </Badge>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={disabled}
            aria-label={`${rank ? "Unrank" : "Rank"} ${label}`}
            onClick={onToggle}
          >
            {rank ? "Unrank" : "Rank"}
          </Button>
        </div>
      )}
    </li>
  );
}

export function ServicePreferenceEditor({
  preference,
  inventory,
  enrichInventory = (items) => items,
  viewMode,
  blocked,
  onClose,
  onDirtyChange,
  reloadPreference,
  refreshInventory,
}: {
  readonly preference: ServicePreference;
  readonly inventory: readonly KeyInfo[];
  readonly enrichInventory?: (
    inventory: readonly KeyInfo[],
  ) => readonly KeyInfo[];
  readonly viewMode: ViewMode;
  readonly blocked: boolean;
  readonly onClose: () => void;
  readonly onDirtyChange: (dirty: boolean) => void;
  readonly reloadPreference: () => Promise<ServicePreference>;
  readonly refreshInventory: () => Promise<readonly KeyInfo[]>;
}) {
  const identity = useAuthStore((state) => state.user?.id);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const stillCurrent = () =>
    mounted.current &&
    identity != null &&
    useAuthStore.getState().user?.id === identity;
  const [items, setItems] = useState(() => initialItems(preference, inventory));
  const [activeId, setActiveId] = useState<string | null>(null);
  const [failure, setFailure] = useState<
    "conflict" | "stale" | "network" | null
  >(null);
  const [message, setMessage] = useState("");
  const [recovering, setRecovering] = useState(false);
  const [editorInventory, setEditorInventory] = useState(inventory);
  const displayInventory = enrichInventory(editorInventory);
  const save = useSaveServicePreference();
  const form = useAppForm<ServicePreferenceRequest>({
    resolver: zodResolver(servicePreferenceRequestSchema),
    defaultValues: {
      ordered: orderedIds(items),
      expected_version: preference.version,
    },
  });
  const dirty = form.formState.isDirty;
  const busy = recovering || save.isPending;
  const root = useRef<HTMLFormElement>(null);
  const needsInitialFocus = useRef(true);
  useEffect(() => {
    if (blocked || !needsInitialFocus.current) return;
    const frame = requestAnimationFrame(() => {
      const handle = root.current?.querySelector<HTMLButtonElement>("button");
      if (handle && !handle.disabled) {
        handle.focus();
        needsInitialFocus.current = false;
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [blocked]);
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const label = (id: string | number) =>
    id === DIVIDER
      ? "Unranked divider"
      : (editorInventory.find((item) => item.id === id)?.label ?? "Service");

  function update(next: string[]) {
    if (orderedIds(next).length > MAX_ORDERED_SERVICES) {
      setMessage(
        "You can rank at most 200 services. Unrank a service before adding another.",
      );
      return;
    }
    setItems(next);
    form.setValue("ordered", orderedIds(next));
    setMessage("");
  }
  async function persist(body: ServicePreferenceRequest) {
    if (!stillCurrent()) return;
    try {
      await save.mutateAsync(servicePreferenceRequestSchema.parse(body));
      if (!stillCurrent()) return;
      toast.success("Preference order saved");
      onClose();
    } catch (error) {
      if (!stillCurrent()) return;
      setFailure(
        error instanceof ApiError && error.status === 409
          ? "conflict"
          : error instanceof ApiError && error.status === 400
            ? "stale"
            : "network",
      );
    }
  }
  async function submit(body: ServicePreferenceRequest) {
    if (blocked || busy) return;
    await persist(body);
  }
  async function recover(overwrite: boolean) {
    setRecovering(true);
    try {
      const current = await reloadPreference();
      if (!stillCurrent()) return;
      if (overwrite) {
        form.setValue("expected_version", current.version, {
          shouldDirty: false,
          shouldTouch: false,
        });
        await persist({
          ordered: form.getValues("ordered"),
          expected_version: current.version,
        });
      } else {
        const fresh = await refreshInventory();
        if (!stillCurrent()) return;
        setEditorInventory(fresh);
        const next = initialItems(current, fresh);
        setItems(next);
        form.reset({
          ordered: orderedIds(next),
          expected_version: current.version,
        });
        setFailure(null);
      }
    } catch {
      if (stillCurrent())
        setMessage(
          "Could not reload the current order. Your edits are kept. Retry recovery.",
        );
    } finally {
      if (stillCurrent()) setRecovering(false);
    }
  }
  async function refreshStale() {
    setRecovering(true);
    try {
      const fresh = await refreshInventory();
      if (!stillCurrent()) return;
      const visible = new Set(fresh.map((item) => item.id));
      const next = items.filter((id) => id === DIVIDER || visible.has(id));
      next.push(
        ...fresh
          .filter((item) => !next.includes(item.id))
          .map((item) => item.id),
      );
      setEditorInventory(fresh);
      update(next);
      setFailure(null);
      setMessage(
        "Inventory refreshed. Unavailable services were removed. Review the order and save again.",
      );
    } catch {
      if (stillCurrent())
        setMessage(
          "Could not refresh services. Your edits are kept. Retry inventory refresh.",
        );
    } finally {
      if (stillCurrent()) setRecovering(false);
    }
  }
  return (
    <form
      ref={root}
      onSubmit={form.handleSubmit(submit)}
      className="min-w-0 space-y-4"
      aria-label="Service preference order"
    >
      <p className="text-12 text-muted-foreground">
        Editing your complete inventory · {editorInventory.length}{" "}
        {editorInventory.length === 1 ? "connection" : "connections"}. Filters
        and saved views are not applied here and are not changed.
      </p>
      <div className="flex items-start gap-3 rounded-xl border border-primary/15 bg-primary/[0.04] px-4 py-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Info className="size-4" aria-hidden="true" />
        </span>
        <p className="text-12 text-muted-foreground">
          Drag the handle to set the order agents see first when tools tie on
          relevance. Keyboard: focus a handle, press Space, use the arrow keys,
          press Space again. Escape cancels a drag. Items below the divider are
          unranked. This does not choose which connection runs a request: that
          is the slug the agent calls, pool priority or rotation, and the
          personal → organization → platform credential cascade.
        </p>
      </div>
      {message && (
        <p role="status" className="text-12 text-muted-foreground">
          {message}
        </p>
      )}
      {failure === "conflict" && (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-xl border p-4"
        >
          <p>Your preference order changed in another tab.</p>
          <Button
            type="button"
            variant="outline"
            disabled={busy}
            onClick={() => void recover(false)}
          >
            Reload order
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={busy}
            onClick={() => void recover(true)}
          >
            Overwrite
          </Button>
        </div>
      )}
      {failure === "stale" && (
        <ErrorBanner
          message="Some services are no longer available. Refresh services before saving again."
          onRetry={() => void refreshStale()}
        />
      )}
      {failure === "network" && (
        <ErrorBanner
          message="Could not save preference order. Your edits are kept."
          onRetry={() => void submit(form.getValues())}
        />
      )}
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        accessibility={{
          announcements: {
            onDragStart: ({ active }) =>
              `Picked up ${label(active.id)}, position ${items.indexOf(String(active.id)) + 1} of ${items.length}`,
            onDragOver: ({ over }) =>
              over
                ? `Moved to position ${items.indexOf(String(over.id)) + 1}`
                : undefined,
            onDragEnd: ({ over }) =>
              over
                ? `Dropped at position ${items.indexOf(String(over.id)) + 1}`
                : "Cancelled",
            onDragCancel: () => "Cancelled",
          },
        }}
        onDragStart={({ active }) => setActiveId(String(active.id))}
        onDragCancel={() => setActiveId(null)}
        onDragEnd={({ active, over }) => {
          setActiveId(null);
          if (over && active.id !== over.id)
            update(
              arrayMove(
                items,
                items.indexOf(String(active.id)),
                items.indexOf(String(over.id)),
              ),
            );
        }}
      >
        <SortableContext items={items} strategy={verticalListSortingStrategy}>
          <ul className="space-y-2" aria-label="Ranked and unranked services">
            {items.map((id, index) => (
              <SortableItem
                key={id}
                id={id}
                item={displayInventory.find((item) => item.id === id)}
                rank={index < items.indexOf(DIVIDER) ? index + 1 : undefined}
                viewMode={viewMode}
                disabled={busy || blocked}
                onToggle={() => {
                  const next = items.filter((value) => value !== id);
                  const divider = next.indexOf(DIVIDER);
                  next.splice(
                    divider + (index < items.indexOf(DIVIDER) ? 1 : 0),
                    0,
                    id,
                  );
                  update(next);
                }}
              />
            ))}
          </ul>
        </SortableContext>
        <DragOverlay>
          {activeId && (
            <div className="rounded-xl border bg-card p-4 shadow-lg">
              {label(activeId)}
            </div>
          )}
        </DragOverlay>
      </DndContext>
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={busy}
          onClick={() => {
            form.reset();
            onClose();
          }}
        >
          Cancel
        </Button>
        <Button
          type="submit"
          variant="primary"
          isLoading={save.isPending}
          disabled={
            !dirty ||
            blocked ||
            busy ||
            failure === "stale" ||
            failure === "conflict"
          }
        >
          Save
        </Button>
      </div>
    </form>
  );
}
