import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { filterError, newView } from "@/lib/usage-analytics";
import {
  workspaceConfigSchema,
  workspaceResponseSchema,
  type WorkspaceConfig,
  type WorkspaceResponse,
} from "@/schemas/usage-analytics";

const draftKey = (userId: string, demo: boolean) =>
  `nyxid:usage-workspace:v1:${demo ? "demo:" : ""}${userId}`;
function readDemo(userId: string): WorkspaceResponse {
  try {
    const raw = localStorage.getItem(`${draftKey(userId, true)}:saved`);
    if (raw) return workspaceResponseSchema.parse(JSON.parse(raw));
  } catch {
    /* Damaged sample settings can be recreated independently of live settings. */
  }
  return { revision: 0, config: null };
}
export function useUsageWorkspace(userId: string, demo = false) {
  return useQuery({
    queryKey: ["usage-workspace", userId, demo],
    queryFn: async (): Promise<WorkspaceResponse> =>
      demo
        ? readDemo(userId)
        : workspaceResponseSchema.parse(
            await api.get<unknown>("/admin/usage/workspace"),
          ),
    staleTime: 0,
    refetchOnWindowFocus: false,
    retry: false,
  });
}
function readDraft(
  key: string,
  initial: WorkspaceResponse,
): {
  config: WorkspaceConfig;
  pending: WorkspaceConfig | null;
  backup: string | null;
} {
  const fallback = initial.config ?? {
    version: 1 as const,
    draft: newView(),
    saved_views: [],
  };
  try {
    const raw = sessionStorage.getItem(key) ?? localStorage.getItem(key);
    if (raw) {
      const parsed = workspaceResponseSchema.safeParse(JSON.parse(raw));
      if (parsed.success && parsed.data.config) {
        const same =
          JSON.stringify(parsed.data.config) === JSON.stringify(initial.config);
        const needsReview =
          !same &&
          (parsed.data.revision !== initial.revision ||
            Boolean(filterError(parsed.data.config.draft.filters)));
        return {
          config: needsReview ? fallback : parsed.data.config,
          pending: needsReview ? parsed.data.config : null,
          backup: raw,
        };
      }
    }
  } catch {
    /* The server remains authoritative when recovery storage is unavailable. */
  }
  return { config: fallback, pending: null, backup: null };
}

export function useAutosavedWorkspace(
  initial: WorkspaceResponse,
  userId: string,
  editable: boolean,
  demo = false,
) {
  const key = draftKey(userId, demo);
  const [recovery] = useState(() =>
    editable
      ? readDraft(key, initial)
      : {
          config: initial.config ?? {
            version: 1 as const,
            draft: newView(),
            saved_views: [],
          },
          pending: null,
          backup: null,
        },
  );
  const [config, setConfigState] = useState<WorkspaceConfig>(recovery.config);
  const [acknowledged, setAcknowledged] = useState(
    JSON.stringify(initial.config),
  );
  const [pendingRecovery, setPendingRecovery] = useState(recovery.pending);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [storageError, setStorageError] = useState(false);
  const revision = useRef(initial.revision);
  const inFlight = useRef(false);
  const latestConfig = useRef(config);
  const backup = useRef(recovery.backup);
  const [writer] = useState(() => crypto.randomUUID());
  const serialized = JSON.stringify(config);
  const dirty = serialized !== acknowledged;
  const validation = workspaceConfigSchema.safeParse(config);
  const validationError = validation.success
    ? null
    : validation.error.issues[0]?.message;
  const valid = validation.success && !filterError(config.draft.filters);
  function setConfig(next: WorkspaceConfig) {
    if (pendingRecovery) return;
    setConfigState(next);
    latestConfig.current = next;
    if (!editable) return;
    persistDraft(next);
  }
  function persistDraft(next: WorkspaceConfig) {
    try {
      const raw = JSON.stringify({
        revision: revision.current,
        config: next,
        writer,
      });
      sessionStorage.setItem(key, raw);
      localStorage.setItem(key, raw);
      backup.current = raw;
      setStorageError(false);
    } catch {
      setStorageError(true);
    }
  }
  useEffect(() => {
    if (
      !editable ||
      !dirty ||
      !valid ||
      error ||
      pendingRecovery ||
      inFlight.current
    )
      return;
    const timer = window.setTimeout(() => {
      inFlight.current = true;
      setSaving(true);
      const savedConfig = config;
      const save = demo
        ? Promise.resolve().then(() => {
            const response = {
              revision: revision.current + 1,
              config: savedConfig,
            };
            localStorage.setItem(`${key}:saved`, JSON.stringify(response));
            return response;
          })
        : api
            .put<unknown>("/admin/usage/workspace", {
              revision: revision.current,
              config: savedConfig,
            })
            .then((response) => workspaceResponseSchema.parse(response));
      void save
        .then((response) => {
          revision.current = response.revision;
          setAcknowledged(JSON.stringify(savedConfig));
          try {
            const pending =
              JSON.stringify(latestConfig.current) !==
              JSON.stringify(savedConfig)
                ? JSON.stringify({
                    revision: response.revision,
                    config: latestConfig.current,
                    writer,
                  })
                : null;
            for (const storage of [sessionStorage, localStorage]) {
              if (storage.getItem(key) !== backup.current) continue;
              if (pending) storage.setItem(key, pending);
              else storage.removeItem(key);
            }
            backup.current = pending;
            setStorageError(false);
          } catch {
            setStorageError(true);
          }
        })
        .catch((cause: unknown) => {
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not save this workspace.",
          );
        })
        .finally(() => {
          inFlight.current = false;
          setSaving(false);
        });
    }, 600);
    return () => window.clearTimeout(timer);
  }, [
    config,
    dirty,
    valid,
    error,
    editable,
    demo,
    key,
    acknowledged,
    saving,
    writer,
    pendingRecovery,
  ]);
  useEffect(() => {
    if (!editable || dirty || pendingRecovery || !backup.current) return;
    try {
      for (const storage of [sessionStorage, localStorage]) {
        if (storage.getItem(key) === backup.current) storage.removeItem(key);
      }
      backup.current = null;
    } catch {
      // A saved draft is safe even if an obsolete recovery copy cannot be removed.
    }
  }, [dirty, editable, key, pendingRecovery]);
  useEffect(() => {
    const preventLoss = (event: BeforeUnloadEvent) => {
      if (dirty && storageError) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", preventLoss);
    return () => window.removeEventListener("beforeunload", preventLoss);
  }, [dirty, storageError]);
  async function reload(overwrite = false) {
    if (inFlight.current) return;
    try {
      const remote = demo
        ? readDemo(userId)
        : workspaceResponseSchema.parse(
            await api.get<unknown>("/admin/usage/workspace"),
          );
      revision.current = remote.revision;
      setAcknowledged(JSON.stringify(remote.config));
      const next = overwrite
        ? (pendingRecovery ?? latestConfig.current)
        : (remote.config ?? {
            version: 1 as const,
            draft: newView(),
            saved_views: [],
          });
      setConfigState(next);
      latestConfig.current = next;
      setPendingRecovery(null);
      if (JSON.stringify(next) !== JSON.stringify(remote.config))
        persistDraft(next);
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not reload workspace.",
      );
    }
  }
  return {
    config,
    setConfig,
    saving,
    dirty,
    error,
    storageError,
    valid,
    validationError,
    pendingRecovery,
    retry: () => setError(null),
    reload,
  };
}
