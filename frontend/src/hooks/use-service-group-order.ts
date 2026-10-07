import { useEffect, useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { ApiError } from "@/lib/api-client";
import { useKeys } from "@/hooks/use-keys";
import {
  SERVICE_ORDER_UNAVAILABLE,
  useSaveServiceGroupOrder,
  useServicePreference,
} from "@/hooks/use-service-preference";
import {
  servicePreferenceGroupRequestSchema,
  servicePreferenceResponseSchema,
  type ServicePreference,
  type ServicePreferenceGroupRequest,
} from "@/schemas/service-preference";
import { useAuthStore } from "@/stores/auth-store";
import type { ServiceConnectionGroup } from "@/lib/service-groups";
import type { KeyInfo } from "@/types/keys";

function groupItems(group: string, inventory: readonly KeyInfo[]) {
  return inventory.filter(
    (key) => `catalog:${key.catalog_service_id}` === group,
  );
}
function initialIds(
  preference: ServicePreference,
  group: string,
  inventory: readonly KeyInfo[],
) {
  const stored =
    preference.groups.find((row) => row.group === group)?.ordered ?? [];
  const available = new Set(inventory.map((key) => key.id));
  const saved = stored.filter((id) => available.has(id));
  return {
    saved,
    display: [
      ...saved,
      ...inventory
        .filter((key) => !saved.includes(key.id))
        .map((key) => key.id),
    ],
  };
}
type Draft = {
  group: string;
  context: ServiceConnectionGroup;
  name: string;
  identity: string;
  token: number;
  ids: string[];
  members: readonly KeyInfo[];
  newIds: string[];
};

export function useServiceGroupOrder(inventory: readonly KeyInfo[]) {
  const identity = useAuthStore((state) => state.user?.id);
  const preference = useServicePreference();
  const keys = useKeys();
  const mutation = useSaveServiceGroupOrder();
  const form = useAppForm<ServicePreferenceGroupRequest>({
    resolver: zodResolver(servicePreferenceGroupRequestSchema),
    defaultValues: { ordered: [], expected_version: 0 },
  });
  const [draft, setDraft] = useState<Draft | null>(null);
  const [failure, setFailure] = useState<
    "conflict" | "stale" | "capacity" | "network" | null
  >(null);
  const [message, setMessage] = useState("");
  const [operation, setOperation] = useState<number | null>(null);
  const [focusGroup, setFocusGroup] = useState<string | null>(null);
  const [previousIdentity, setPreviousIdentity] = useState(identity);
  const live = useRef<Draft | null>(null);
  const sequence = useRef(0);
  const lastSnapshot = useRef(0);
  const retainedInventory = useRef(inventory);
  if (!keys.isError && keys.data) retainedInventory.current = inventory;
  const mounted = useRef(true);
  const replaceDraft = (next: Draft | null) => {
    live.current = next;
    setDraft(next);
  };
  if (previousIdentity !== identity) {
    live.current = null;
    retainedInventory.current = [];
    setPreviousIdentity(identity);
    setDraft(null);
    setFailure(null);
    setFocusGroup(null);
    setMessage("");
    setOperation(null);
  }
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      live.current = null;
    };
  }, []);
  const current = draft?.identity === identity ? draft : null;
  const valid = (captured: Draft) =>
    mounted.current &&
    live.current?.token === captured.token &&
    useAuthStore.getState().user?.id === captured.identity;
  const busy = current != null && operation === current.token;
  const readPending =
    keys.isLoading ||
    keys.isFetching ||
    preference.isLoading ||
    preference.isFetching;
  const readError = keys.isError || preference.isError;
  const unavailable = preference.data === SERVICE_ORDER_UNAVAILABLE;
  const reason = unavailable
    ? "Saving agent order requires the backend update"
    : readError
      ? "Agent order could not be loaded · Retry"
      : readPending || !preference.data
        ? "Loading agent order"
        : current
          ? "Another group is being ordered"
          : undefined;
  const dirty = current != null && form.formState.isDirty;

  useEffect(() => {
    const captured = live.current;
    if (
      !captured ||
      keys.isError ||
      keys.isFetching ||
      !keys.data ||
      lastSnapshot.current === keys.dataUpdatedAt
    )
      return;
    lastSnapshot.current = keys.dataUpdatedAt;
    const fresh = groupItems(captured.group, inventory);
    const available = new Set(fresh.map((key) => key.id));
    const added = fresh
      .filter((key) => !captured.ids.includes(key.id))
      .map((key) => key.id);
    const removed = captured.ids.some((id) => !available.has(id));
    const ids = [...captured.ids.filter((id) => available.has(id)), ...added];
    const ordered = form.getValues("ordered");
    if (removed || added.length) {
      form.setValue(
        "ordered",
        ordered.length
          ? [...ordered.filter((id) => available.has(id)), ...added]
          : [],
        { shouldDirty: form.formState.isDirty, shouldTouch: false },
      );
      setMessage(
        removed
          ? "Connections refreshed. Unavailable rows were removed; new rows were appended. Your order and version are kept."
          : "New connections were appended. Your order and version are kept.",
      );
    }
    const next = {
      ...captured,
      ids,
      members: fresh,
      newIds: [...captured.newIds.filter((id) => available.has(id)), ...added],
    };
    live.current = next;
    setDraft(next);
  }, [
    inventory,
    keys.data,
    keys.dataUpdatedAt,
    keys.isError,
    keys.isFetching,
    form,
  ]);

  useEffect(() => {
    if (!focusGroup) return;
    const cancel = () => setFocusGroup(null);
    window.addEventListener("pointerdown", cancel);
    window.addEventListener("keydown", cancel);
    window.addEventListener("wheel", cancel);
    return () => {
      window.removeEventListener("pointerdown", cancel);
      window.removeEventListener("keydown", cancel);
      window.removeEventListener("wheel", cancel);
    };
  }, [focusGroup]);
  const guarded = () => {
    const captured = live.current;
    if (!captured) return true;
    if (
      operation === captured.token ||
      (form.formState.isDirty &&
        !window.confirm("Discard unsaved agent order?"))
    )
      return false;
    replaceDraft(null);
    setFailure(null);
    setMessage("");
    return true;
  };
  const close = (captured: Draft) => {
    if (!valid(captured)) return;
    setFocusGroup(captured.group);
    replaceDraft(null);
    setFailure(null);
    setMessage("");
  };
  const update = (ids: string[]) => {
    if (!current || busy) return;
    replaceDraft({ ...current, ids });
    form.setValue("ordered", ids);
    setMessage("");
  };
  const persist = async (
    captured: Draft,
    body: ServicePreferenceGroupRequest,
  ) => {
    if (!valid(captured)) return;
    const result = servicePreferenceGroupRequestSchema.safeParse(body);
    if (!result.success) {
      form.setError("ordered", {
        type: "validate",
        message: result.error.issues[0]?.message ?? "Invalid agent order",
      });
      return;
    }
    const parsed = result.data;
    // The mutation also checks live authentication immediately before PUT.
    setOperation(captured.token);
    try {
      if (!valid(captured)) return;
      await mutation.save(captured.group, parsed);
      if (!valid(captured)) return;
      toast.success(`Agent order saved for ${captured.name}`);
      close(captured);
    } catch (error) {
      if (!valid(captured)) return;
      setFailure(
        error instanceof ApiError && error.status === 409
          ? "conflict"
          : error instanceof ApiError && error.status === 400
            ? /capacity|storage is full/i.test(error.message)
              ? "capacity"
              : "stale"
            : "network",
      );
      if (
        error instanceof ApiError &&
        error.status === 400 &&
        /capacity|storage is full/i.test(error.message)
      )
        setMessage(error.message);
    } finally {
      if (valid(captured)) setOperation(null);
    }
  };
  const reloadPreference = async () => {
    const result = await preference.refetch();
    if (
      result.isError ||
      !result.data ||
      result.data === SERVICE_ORDER_UNAVAILABLE
    )
      throw new Error("Could not load agent order");
    return servicePreferenceResponseSchema.parse(result.data);
  };
  const refreshInventory = async () => {
    const result = await keys.refetch();
    if (result.isError || !result.data)
      throw new Error("Could not load connections");
    return result.data;
  };
  const recover = async (overwrite: boolean) => {
    const captured = live.current;
    if (!captured || busy) return;
    setOperation(captured.token);
    try {
      const latest = await reloadPreference();
      if (!valid(captured)) return;
      const fresh = groupItems(captured.group, await refreshInventory());
      if (!valid(captured)) return;
      const available = new Set(fresh.map((key) => key.id));
      if (overwrite) {
        const previous = form.getValues("ordered");
        const ordered = previous.filter((id) => available.has(id));
        if (previous.length)
          ordered.push(
            ...fresh
              .filter((key) => !ordered.includes(key.id))
              .map((key) => key.id),
          );
        form.setValue("expected_version", latest.version, {
          shouldDirty: false,
          shouldTouch: false,
        });
        await persist(captured, { ordered, expected_version: latest.version });
      } else {
        const next = initialIds(latest, captured.group, fresh);
        replaceDraft({
          ...captured,
          members: fresh,
          ids: next.display,
          newIds: [],
        });
        form.reset({ ordered: next.saved, expected_version: latest.version });
        setFailure(null);
        setMessage("");
      }
    } catch {
      if (valid(captured))
        setMessage(
          "Could not reload agent order. Your edits are kept. Retry recovery.",
        );
    } finally {
      if (valid(captured)) setOperation(null);
    }
  };
  const refreshStale = async () => {
    const captured = live.current;
    if (!captured || busy) return;
    setOperation(captured.token);
    try {
      const fresh = groupItems(captured.group, await refreshInventory());
      if (!valid(captured)) return;
      const available = new Set(fresh.map((key) => key.id));
      const ids = captured.ids.filter((id) => available.has(id));
      ids.push(
        ...fresh.filter((key) => !ids.includes(key.id)).map((key) => key.id),
      );
      replaceDraft({ ...captured, members: fresh, ids });
      const previous = form.getValues("ordered");
      const ordered = previous.filter((id) => available.has(id));
      if (previous.length)
        ordered.push(
          ...fresh
            .filter((key) => !ordered.includes(key.id))
            .map((key) => key.id),
        );
      form.setValue("ordered", ordered);
      setFailure(null);
      setMessage(
        "Connections refreshed. Unavailable rows were removed. Review the order and save again.",
      );
    } catch {
      if (valid(captured))
        setMessage(
          "Could not refresh connections. Your edits are kept. Retry inventory refresh.",
        );
    } finally {
      if (valid(captured)) setOperation(null);
    }
  };
  const releaseHidden = async () => {
    const captured = live.current;
    if (
      !captured ||
      busy ||
      !window.confirm(
        "Release preferences for connections you cannot currently access? Their orders will need to be set again if access returns. Visible connection orders are kept.",
      )
    )
      return;
    setOperation(captured.token);
    try {
      const latest = await reloadPreference();
      if (!valid(captured)) return;
      const released = await mutation.release(latest.version);
      if (!valid(captured)) return;
      form.setValue("expected_version", released.version, {
        shouldDirty: false,
        shouldTouch: false,
      });
      setFailure(null);
      setMessage(
        "Unavailable preferences released. Your draft is kept. Retry save when ready.",
      );
    } catch (error) {
      if (valid(captured)) {
        setFailure(
          error instanceof ApiError && error.status === 409
            ? "conflict"
            : "capacity",
        );
        setMessage(
          "Could not release unavailable preferences. Your draft is kept. Retry release.",
        );
      }
    } finally {
      if (valid(captured)) setOperation(null);
    }
  };
  const members =
    current?.members.map((key) => ({
      ...key,
      ...inventory.find((item) => item.id === key.id),
    })) ?? [];
  const connections = current
    ? current.ids.flatMap((id) => {
        const key = members.find((key) => key.id === id);
        return key ? [key] : [];
      })
    : [];
  const requestIds = form.watch("ordered");
  const savedOrder = (group: string) =>
    preference.data && preference.data !== SERVICE_ORDER_UNAVAILABLE
      ? (preference.data.groups.find((row) => row.group === group)?.ordered ??
        [])
      : [];
  return {
    groupId: current?.group,
    editingGroup: current ? { ...current.context, connections } : undefined,
    inventory: current && keys.isError ? retainedInventory.current : inventory,
    connections,
    validationError: form.formState.errors.ordered?.message,
    ordered: requestIds,
    savedOrder,
    dirty,
    busy,
    failure,
    message,
    reason,
    unavailable,
    readError,
    readPending,
    newIds: current?.newIds ?? [],
    blocked: readPending || readError || unavailable,
    focusGroup,
    clearFocus: () => setFocusGroup(null),
    guard: guarded,
    start: (group: ServiceConnectionGroup) => {
      if (
        reason ||
        !identity ||
        !preference.data ||
        preference.data === SERVICE_ORDER_UNAVAILABLE ||
        group.connections.length < 2 ||
        !group.id.startsWith("catalog:")
      )
        return;
      const next = initialIds(preference.data, group.id, group.connections);
      form.reset({
        ordered: next.saved,
        expected_version: preference.data.version,
      });
      lastSnapshot.current = keys.dataUpdatedAt;
      replaceDraft({
        group: group.id,
        context: group,
        name: group.name,
        identity,
        token: ++sequence.current,
        ids: next.display,
        members: group.connections,
        newIds: [],
      });
      setFailure(null);
      setMessage("");
    },
    update,
    reset: () => {
      if (
        current &&
        !busy &&
        window.confirm(
          "Reset this service's agent order to default server discovery order?",
        )
      ) {
        replaceDraft({ ...current, ids: current.members.map((key) => key.id) });
        form.setValue("ordered", []);
        setFailure(null);
        setMessage("");
      }
    },
    moveDisabled: () =>
      update(
        [
          ...connections.filter((key) => key.is_active),
          ...connections.filter((key) => !key.is_active),
        ].map((key) => key.id),
      ),
    save: form.handleSubmit(
      async (body) => {
        const captured = live.current;
        if (captured && !busy && !readPending && !readError && !unavailable)
          await persist(captured, body);
      },
      () =>
        setMessage(
          "At most 200 connections can have a saved agent order across all services. Reset this group to default, or reduce the group before saving.",
        ),
    ),
    cancel: () => {
      if (current && !busy) close(current);
    },
    retrySave: async () => {
      const captured = live.current;
      if (captured && !busy && !readPending && !readError && !unavailable)
        await persist(captured, form.getValues());
    },
    recover,
    refreshStale,
    releaseHidden,
    retryRead: async () => {
      await Promise.all([preference.refetch(), keys.refetch()]);
    },
  };
}
export type ServiceGroupOrder = ReturnType<typeof useServiceGroupOrder>;
