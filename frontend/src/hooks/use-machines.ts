import type { z } from "zod";
import { assertRemoteMachineAdmission } from "@/hooks/use-remote-machine-admission";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type MutateOptions,
  type UseMutationOptions,
  type UseQueryResult,
} from "@tanstack/react-query";
import { useCallback, useEffect, useRef } from "react";
import { api, apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  REMOTE_MACHINE_PROFILE,
  remoteMachineActionRequestSchema,
  remoteMachineCreateRequestSchema,
  remoteMachineCreateResponseSchema,
  remoteMachineConfirmationDecisionSchema,
  remoteMachineBoundConfirmationResponseSchema,
  remoteMachineJobCancellationSchema,
  remoteMachineConfirmationsSchema,
  remoteMachineEntitlementSchema,
  remoteMachineEventPageSchema,
  remoteMachineListResponseSchema,
  remoteMachineJobsSchema,
  remoteMachineJobOutputSchema,
  remoteMachineOperationResponseSchema,
  remoteMachineDeleteRetryRequestSchema,
  remoteMachineQuoteRequestSchema,
  remoteMachineQuoteSchema,
  remoteMachineResponseSchema,
  remoteMachineRuntimeBillingSchema,
  remoteMachineBillingRunsSchema,
  remoteMachineOwnerHistorySchema,
  remoteMachineOwnerConfirmationsSchema,
  type RemoteMachineActionRequest,
  type RemoteMachineCreateRequest,
  type RemoteMachineConfirmationDecision,
} from "@/schemas/machines";
import type {
  MachineChoices,
  MachineSetup,
  MachineUpdateStatus,
} from "@/schemas/machines";

const REMOTE_MACHINES_KEY = ["remote-machines"] as const;
const remoteMachineOwnerKey = (owner: string | undefined) =>
  [...REMOTE_MACHINES_KEY, owner] as const;
const remoteMachineKey = (owner: string | undefined, id: string | undefined) =>
  [...remoteMachineOwnerKey(owner), id] as const;
const remoteMachineOperationKey = (
  owner: string | undefined,
  id: string | undefined,
) => ["remote-machine-operations", owner, id] as const;

function assertRemoteMachineOwner(
  owner: string | undefined,
): asserts owner is string {
  if (!owner || useAuthStore.getState().user?.id !== owner) {
    throw new Error(
      "Your account changed. Reload before managing this Machine.",
    );
  }
}

function remoteMachineAuthority(owner: string | undefined, admission = false) {
  return {
    authorityGuard: () => {
      assertRemoteMachineOwner(owner);
      if (admission) assertRemoteMachineAdmission(owner);
    },
  };
}

function safeRemoteMachineQuery<Data>(query: UseQueryResult<Data, Error>) {
  // Rejected live reads must never leave an old authorized view actionable.
  return { ...query, data: query.isError ? undefined : query.data };
}

/** Capture ownership in variables before any async mutation-cache hook runs. */
function useRemoteMachineOwnerMutation<
  Data,
  Variables extends { readonly owner: string },
>(options: UseMutationOptions<Data, Error, Variables>) {
  const owner = useAuthStore((state) => state.user?.id);
  const mutation = useMutation(options);
  const dispatch = mutation.mutate;
  const dispatchAsync = mutation.mutateAsync;
  const mutate = useCallback(
    (
      variables: Omit<Variables, "owner">,
      callbacks?: MutateOptions<Data, Error, Omit<Variables, "owner">>,
    ) => {
      assertRemoteMachineOwner(owner);
      dispatch({ ...variables, owner } as Variables, callbacks);
    },
    [dispatch, owner],
  );
  const mutateAsync = useCallback(
    async (
      variables: Omit<Variables, "owner">,
      callbacks?: MutateOptions<Data, Error, Omit<Variables, "owner">>,
    ) => {
      assertRemoteMachineOwner(owner);
      return dispatchAsync({ ...variables, owner } as Variables, callbacks);
    },
    [dispatchAsync, owner],
  );
  return {
    ...mutation,
    mutate,
    mutateAsync,
  };
}

function idempotencyHeaders(key: string) {
  if (!key.trim() || new TextEncoder().encode(key).length > 256) {
    throw new Error(
      "A stable Idempotency-Key between 1 and 256 bytes is required",
    );
  }
  return { "Idempotency-Key": key };
}

export function useRemoteMachines(enabled = true, includeDeleted = false) {
  const owner = useAuthStore((state) => state.user?.id);
  const query = safeRemoteMachineQuery(
    useQuery({
      queryKey: includeDeleted
        ? [...remoteMachineOwnerKey(owner), "retained"]
        : remoteMachineOwnerKey(owner),
      queryFn: async () => {
        const machines: z.infer<
          typeof remoteMachineListResponseSchema
        >["machines"] = [];
        const seen = new Set<string>();
        let cursor: string | undefined;
        for (let page = 0; page < 100; page += 1) {
          const params = new URLSearchParams();
          if (includeDeleted) params.set("include_deleted", "true");
          if (cursor) {
            params.set("cursor", cursor);
            params.set("limit", "100");
          }
          const result = remoteMachineListResponseSchema.parse(
            await api.get<unknown>(
              `/remote-machines${params.size ? `?${params}` : ""}`,
              remoteMachineAuthority(owner),
            ),
          );
          machines.push(
            ...result.machines.filter(
              (row) => !machines.some((old) => old.node_id === row.node_id),
            ),
          );
          if (!result.next_cursor)
            return { machines, complete: result.next_cursor === null };
          if (seen.has(result.next_cursor))
            throw new Error("Machine discovery returned a repeated cursor");
          seen.add(result.next_cursor);
          cursor = result.next_cursor;
        }
        throw new Error(
          "Machine discovery exceeded its bounded page budget. Try again later.",
        );
      },
      enabled: enabled && Boolean(owner),
      retry: false,
      refetchInterval: 5000,
    }),
  );
  return {
    ...query,
    data: query.data?.machines,
    discoveryComplete: query.data?.complete === true,
  };
}

export function useRemoteMachine(id: string | undefined) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: remoteMachineKey(owner, id),
      queryFn: async () =>
        remoteMachineResponseSchema.parse(
          await api.get<unknown>(
            `/remote-machines/${encodeURIComponent(id ?? "")}`,
            remoteMachineAuthority(owner),
          ),
        ).machine,
      enabled: Boolean(owner && id),
      retry: false,
      refetchInterval: (query) =>
        query.state.data?.resource_state === "deleted" ? false : 5000,
    }),
  );
}

export function useRemoteMachineEntitlement(
  enabled = true,
  profileVersion: string = REMOTE_MACHINE_PROFILE,
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: ["remote-machine-entitlement", owner, profileVersion],
      queryFn: async () =>
        remoteMachineEntitlementSchema.parse(
          await api.get<unknown>(
            `/remote-machine-entitlement?${new URLSearchParams({ profile_version: profileVersion })}`,
            remoteMachineAuthority(owner),
          ),
        ),
      enabled: enabled && Boolean(owner),
      retry: false,
      staleTime: 0,
    }),
  );
}

export function useRemoteMachineQuote() {
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      free_slot,
      machine_id,
      profile_version,
      owner,
    }: {
      readonly free_slot: boolean;
      readonly machine_id?: string;
      readonly profile_version?: typeof REMOTE_MACHINE_PROFILE;
      readonly owner: string;
    }) =>
      remoteMachineQuoteSchema.parse(
        await apiClient<unknown>("/remote-machine-quotes", {
          method: "POST",
          body: remoteMachineQuoteRequestSchema.parse({
            free_slot,
            machine_id,
            profile_version,
          }),
          ...remoteMachineAuthority(owner, true),
        }),
      ),
    retry: false,
  });
}

export function useCreateRemoteMachine() {
  const client = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      body,
      idempotencyKey,
      owner,
    }: {
      readonly body: RemoteMachineCreateRequest;
      readonly idempotencyKey: string;
      readonly owner: string;
    }) =>
      remoteMachineCreateResponseSchema.parse(
        await apiClient<unknown>("/remote-machines", {
          method: "POST",
          body: remoteMachineCreateRequestSchema.parse(body),
          headers: idempotencyHeaders(idempotencyKey),
          ...remoteMachineAuthority(owner, true),
        }),
      ),
    // A retry always reuses the caller-owned intent and reviewed body.
    retry: false,
    onSuccess: (response, { owner }) => {
      if (useAuthStore.getState().user?.id !== owner) return;
      client.setQueryData(
        remoteMachineKey(owner, response.machine.node_id),
        response.machine,
      );
      client.setQueryData(
        remoteMachineOperationKey(owner, response.operation.id),
        response.operation,
      );
      void client.invalidateQueries({ queryKey: remoteMachineOwnerKey(owner) });
      void client.invalidateQueries({
        queryKey: ["remote-machine-entitlement", owner],
      });
      void client.invalidateQueries({ queryKey: ["nodes"] });
    },
    onError: (_error, { owner }) => {
      if (useAuthStore.getState().user?.id === owner)
        void client.invalidateQueries({
          queryKey: ["remote-machine-entitlement", owner],
        });
    },
  });
}

export function useRemoteMachineAction() {
  const client = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      machineId,
      body,
      idempotencyKey,
      owner,
    }: {
      readonly machineId: string;
      readonly body: RemoteMachineActionRequest;
      readonly idempotencyKey: string;
      readonly owner: string;
    }) => {
      const request = remoteMachineActionRequestSchema.parse(body);
      const root = `/remote-machines/${encodeURIComponent(machineId)}`;
      const endpoint =
        request.action === "delete"
          ? root
          : request.action === "rekey"
            ? `${root}/rekey`
            : `${root}/actions`;
      return remoteMachineOperationResponseSchema.parse(
        await apiClient<unknown>(endpoint, {
          method: request.action === "delete" ? "DELETE" : "POST",
          body: request,
          headers: idempotencyHeaders(idempotencyKey),
          ...remoteMachineAuthority(
            owner,
            request.action === "resume" || request.action === "rekey",
          ),
        }),
      );
    },
    retry: false,
    onSuccess: (response, { machineId, owner }) => {
      if (useAuthStore.getState().user?.id !== owner) return;
      client.setQueryData(
        remoteMachineOperationKey(owner, response.operation.id),
        response.operation,
      );
      void client.invalidateQueries({
        queryKey: remoteMachineKey(owner, machineId),
      });
      void client.invalidateQueries({ queryKey: remoteMachineOwnerKey(owner) });
      void client.invalidateQueries({
        queryKey: ["remote-machine-entitlement", owner],
      });
      void client.invalidateQueries({ queryKey: ["nodes"] });
    },
    onError: (_error, { machineId, owner }) => {
      // A CAS conflict needs a fresh view. Do not silently change the reviewed
      // revision, regenerate the intent key, or issue a second lifecycle action.
      if (useAuthStore.getState().user?.id === owner)
        void client.invalidateQueries({
          queryKey: remoteMachineKey(owner, machineId),
        });
    },
  });
}

export function useRetryRemoteMachineDelete() {
  const client = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      machineId,
      operationId,
      body,
      idempotencyKey,
      owner,
    }: {
      readonly machineId: string;
      readonly operationId: string;
      readonly body: z.infer<typeof remoteMachineDeleteRetryRequestSchema>;
      readonly idempotencyKey: string;
      readonly owner: string;
    }) => {
      const response = remoteMachineOperationResponseSchema.parse(
        await apiClient<unknown>(
          `/remote-machine-operations/${encodeURIComponent(operationId)}/retry-delete`,
          {
            method: "POST",
            body: remoteMachineDeleteRetryRequestSchema.parse(body),
            headers: idempotencyHeaders(idempotencyKey),
            ...remoteMachineAuthority(owner),
          },
        ),
      );
      if (
        response.operation.id !== operationId ||
        response.operation.machine_id !== machineId ||
        response.operation.action !== "delete"
      ) {
        throw new Error(
          "Deletion retry returned a different operation or Machine",
        );
      }
      return response;
    },
    retry: false,
    onSuccess: (response, { machineId, owner }) => {
      if (useAuthStore.getState().user?.id !== owner) return;
      client.setQueryData(
        remoteMachineOperationKey(owner, response.operation.id),
        response.operation,
      );
      void client.invalidateQueries({
        queryKey: remoteMachineKey(owner, machineId),
      });
      void client.invalidateQueries({ queryKey: remoteMachineOwnerKey(owner) });
    },
    onError: (_error, { machineId, operationId, owner }) => {
      if (useAuthStore.getState().user?.id !== owner) return;
      void client.invalidateQueries({
        queryKey: remoteMachineKey(owner, machineId),
      });
      void client.invalidateQueries({
        queryKey: remoteMachineOperationKey(owner, operationId),
      });
    },
  });
}

export function useRemoteMachineOperation(id: string | undefined) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: remoteMachineOperationKey(owner, id),
      queryFn: async () =>
        remoteMachineOperationResponseSchema.parse(
          await api.get<unknown>(
            `/remote-machine-operations/${encodeURIComponent(id ?? "")}`,
            remoteMachineAuthority(owner),
          ),
        ).operation,
      enabled: Boolean(owner && id),
      retry: false,
      refetchInterval: (query) =>
        ["succeeded", "failed"].includes(query.state.data?.state ?? "")
          ? false
          : 3000,
    }),
  );
}

export function useRemoteMachineEvents(
  machineId: string | undefined,
  cursor?: string,
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: [
        ...remoteMachineKey(owner, machineId),
        "events",
        cursor ?? null,
      ],
      queryFn: async () => {
        const params = new URLSearchParams({ limit: "100" });
        if (cursor) params.set("cursor", cursor);
        const result = remoteMachineEventPageSchema.parse(
          await api.get<unknown>(
            `/remote-machines/${encodeURIComponent(machineId ?? "")}/events?${params}`,
            remoteMachineAuthority(owner),
          ),
        );
        if (result.events.some((event) => event.machine_id !== machineId))
          throw new Error("Timeline returned a different Machine");
        return result;
      },
      enabled: Boolean(owner && machineId),
      retry: false,
      refetchInterval: cursor ? false : 5000,
    }),
  );
}

/** Mounted owner HTTP adapter for P3; missing contracts are never zero balances. */
export function useRemoteMachineRuntimeBilling(
  machineId: string | undefined,
  runId?: string,
  offset = 0,
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: [
        ...remoteMachineKey(owner, machineId),
        "billing",
        runId ?? null,
        offset,
      ],
      queryFn: async () => {
        const params = new URLSearchParams();
        if (runId) params.set("run_id", runId);
        if (offset > 0) params.set("offset", String(offset));
        const query = params.size ? `?${params}` : "";
        return remoteMachineRuntimeBillingSchema.parse(
          await api.get<unknown>(
            `/remote-machines/${encodeURIComponent(machineId ?? "")}/billing${query}`,
            remoteMachineAuthority(owner),
          ),
        );
      },
      enabled: Boolean(owner && machineId),
      retry: false,
      refetchInterval: (query) =>
        query.state.status === "error" || query.state.data?.state === "closed"
          ? false
          : 10000,
    }),
  );
}

/** Numeric backend offsets are represented as strings by the existing pager. */
export function useRemoteMachineBillingRuns(
  machineId: string | undefined,
  cursor?: string,
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: [
        ...remoteMachineKey(owner, machineId),
        "billing",
        "runs",
        cursor ?? null,
      ],
      queryFn: async () => {
        const params = new URLSearchParams({ limit: "25" });
        if (cursor) {
          if (!/^\d+$/.test(cursor) || !Number.isSafeInteger(Number(cursor)))
            throw new Error("Invalid billing history offset");
          params.set("offset", cursor);
        }
        return remoteMachineBillingRunsSchema.parse(
          await api.get<unknown>(
            `/remote-machines/${encodeURIComponent(machineId ?? "")}/billing/runs?${params}`,
            remoteMachineAuthority(owner),
          ),
        );
      },
      enabled: Boolean(owner && machineId),
      retry: false,
      refetchInterval: (query) =>
        query.state.status === "error" || cursor ? false : 10000,
    }),
  );
}

export interface RemoteMachineJobsFilter {
  readonly status?: string;
  readonly from?: string;
  readonly to?: string;
  readonly cursor?: string;
}

export function useRemoteMachineJobs(
  machineId: string | undefined,
  filters: RemoteMachineJobsFilter = {},
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: [...remoteMachineKey(owner, machineId), "jobs", filters],
      queryFn: async () => {
        const params = new URLSearchParams({ limit: "25" });
        if (filters.from) params.set("from", filters.from);
        if (filters.to) params.set("to", filters.to);
        if (filters.status) params.set("state", filters.status);
        if (filters.cursor) params.set("cursor", filters.cursor);
        const history = remoteMachineOwnerHistorySchema.parse(
          await api.get<unknown>(
            `/remote-machines/${encodeURIComponent(machineId ?? "")}/jobs?${params}`,
            remoteMachineAuthority(owner),
          ),
        );
        if (
          history.jobs.some(
            (job) =>
              job.node_id !== machineId ||
              (job.binding.kind === "external" &&
                job.binding.principal.user_id !== owner),
          )
        )
          throw new Error(
            "The server returned history for a different Machine or owner",
          );
        return remoteMachineJobsSchema.parse({
          next_cursor: history.next_cursor,
          jobs: history.jobs.map((job) => ({
            ...job,
            status: job.state,
            subject:
              job.binding.kind === "external"
                ? job.binding.principal
                : {
                    kind: "assistant",
                    user_id: owner,
                    agent_id: job.binding.agent_id ?? null,
                    conversation_id: job.binding.conversation_id ?? null,
                  },
            grant_id:
              job.binding.kind === "external" ? job.binding.grant_id : null,
            started_at: null,
            exit_code: job.exit_code ?? null,
            stop_reason: job.stop_reason ?? null,
            output_availability: job.output_availability ?? "unavailable",
            output_expires_at: null,
            output_reason:
              job.output_query_available === true
                ? "A live read is required to check whether output remains in the original runtime cache."
                : "Output is not queryable in the current runtime. It may have expired, been evicted, or become unavailable after a runtime change.",
            cancellation_available: job.cancellation_available === true,
          })),
        });
      },
      enabled: Boolean(owner && machineId),
      retry: false,
      refetchInterval: (query) =>
        query.state.status === "error" || filters.cursor ? false : 5000,
    }),
  );
}

/** No Query or mutation wrapper: output must not enter persisted app caches. */
export async function readRemoteMachineJobOutput(
  machineId: string,
  jobId: string,
  offsets: {
    readonly output_offset?: number;
    readonly stderr_offset?: number;
  } = {},
  signal?: AbortSignal,
) {
  const owner = useAuthStore.getState().user?.id;
  assertRemoteMachineOwner(owner);
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(offsets)) {
    if (!Number.isSafeInteger(value) || value < 0)
      throw new Error("Output offsets must be nonnegative safe integers");
    params.set(key, String(value));
  }
  const suffix = params.size ? `?${params}` : "";
  const output = remoteMachineJobOutputSchema.parse(
    await apiClient<unknown>(
      `/remote-machines/${encodeURIComponent(machineId)}/jobs/${encodeURIComponent(jobId)}/output${suffix}`,
      {
        signal,
        headers: { "Cache-Control": "no-store", Pragma: "no-cache" },
        ...remoteMachineAuthority(owner),
      },
    ),
  );
  if (output.job_id !== jobId)
    throw new Error("The server returned output for a different job");
  return output;
}

export function useCancelRemoteMachineJob() {
  const client = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      machineId,
      jobId,
      owner,
    }: {
      readonly machineId: string;
      readonly jobId: string;
      readonly owner: string;
    }) => {
      // Cancellation carries no output and cannot resume a job or extend a lease.
      const response = remoteMachineJobCancellationSchema.parse(
        await apiClient<unknown>(
          `/remote-machines/${encodeURIComponent(machineId)}/jobs/${encodeURIComponent(jobId)}/cancel`,
          { method: "POST", body: {}, ...remoteMachineAuthority(owner) },
        ),
      );
      if (response.job_id !== jobId)
        throw new Error("The server returned cancellation for a different job");
      return response;
    },
    retry: false,
    onSuccess: (_response, { machineId, owner }) => {
      if (useAuthStore.getState().user?.id === owner)
        void client.invalidateQueries({
          queryKey: [...remoteMachineKey(owner, machineId), "jobs"],
        });
    },
  });
}

export function useRemoteMachineConfirmations(
  machineId: string | undefined,
  cursor?: string,
) {
  const owner = useAuthStore((state) => state.user?.id);
  return safeRemoteMachineQuery(
    useQuery({
      queryKey: [
        ...remoteMachineKey(owner, machineId),
        "confirmations",
        cursor ?? null,
      ],
      queryFn: async () => {
        const params = new URLSearchParams({
          node_id: machineId ?? "",
          limit: "100",
        });
        if (cursor) params.set("cursor", cursor);
        const result = remoteMachineOwnerConfirmationsSchema.parse(
          await api.get<unknown>(
            `/machine-confirmations?${params}`,
            remoteMachineAuthority(owner),
          ),
        );
        if (
          result.confirmations.some(
            (row) =>
              row.node_id !== machineId || row.principal.user_id !== owner,
          )
        )
          throw new Error(
            "The server returned a confirmation for a different Machine or owner",
          );
        return remoteMachineConfirmationsSchema.parse({
          next_cursor: result.next_cursor,
          confirmations: result.confirmations.map((row) => ({
            ...row,
            subject: row.principal,
            status: row.state,
            review_parameters_available:
              row.review_parameters_available === true,
            approved_at: row.state === "approved" ? row.decided_at : null,
            approved_until: row.state === "approved" ? row.expires_at : null,
          })),
        });
      },
      enabled: Boolean(owner && machineId),
      retry: false,
      refetchInterval: (query) =>
        query.state.status === "error" || cursor ? false : 5000,
    }),
  );
}

export function useDecideRemoteMachineConfirmation() {
  const client = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: async ({
      machineId,
      confirmationId,
      body,
      owner,
    }: {
      readonly machineId: string;
      readonly confirmationId: string;
      readonly body: RemoteMachineConfirmationDecision;
      readonly owner: string;
    }) => {
      const reviewed = remoteMachineConfirmationDecisionSchema.parse(body);
      const response = await apiClient<unknown>(
        `/remote-machines/${encodeURIComponent(machineId)}/confirmations/${encodeURIComponent(confirmationId)}/decision`,
        {
          method: "POST",
          body: reviewed,
          ...remoteMachineAuthority(owner),
        },
      );
      const row =
        remoteMachineBoundConfirmationResponseSchema.parse(
          response,
        ).confirmation;
      if (
        row.id !== confirmationId ||
        row.node_id !== machineId ||
        row.subject.user_id !== owner ||
        row.grant_revision !== reviewed.expected_grant_revision ||
        row.runtime_generation !== reviewed.expected_runtime_generation ||
        row.parameters_digest !== reviewed.parameters_digest
      ) {
        throw new Error(
          "The server returned a decision for a different reviewed confirmation, Machine or owner",
        );
      }
      return row;
    },
    retry: false,
    onSettled: (_response, _error, { machineId, owner }) => {
      if (useAuthStore.getState().user?.id === owner)
        void client.invalidateQueries({
          queryKey: [...remoteMachineKey(owner, machineId), "confirmations"],
        });
    },
  });
}

export function useMachineSetup(id: string | undefined) {
  return useQuery({
    queryKey: ["machine-setup", id],
    queryFn: () => api.get<MachineSetup>(`/machines/setups/${id}`),
    enabled: Boolean(id),
    refetchInterval: (query) =>
      [
        "connected",
        "expired",
        "declined",
        "failed",
        "permissions_missing",
        "offline",
      ].includes(query.state.data?.status ?? "")
        ? false
        : 1000,
  });
}
export function useCreateMachineSetup() {
  return useMutation({
    mutationFn: (choices: MachineChoices) =>
      api.post<MachineSetup>("/machines/setups", choices),
  });
}
// Setup credentials are intentionally never put in the Query cache or persistence.
export async function issueMachineSetup(id: string, choices: MachineChoices) {
  return api.post<{ token: string }>(`/machines/setups/${id}/token`, choices);
}
export function useMachinePairPreview() {
  return useMutation({
    mutationFn: (code: string) =>
      api.post<MachineSetup>("/machines/pair/preview", { code }),
  });
}
export function useMachinePairDecision() {
  return useMutation({
    mutationFn: (body: {
      code: string;
      approve: boolean;
      automatic_updates?: boolean;
    }) => api.post<MachineSetup>("/machines/pair/decide", body),
  });
}
export function useMachineSettings(id: string) {
  const query = useQueryClient();
  return useRemoteMachineOwnerMutation({
    mutationFn: ({
      owner,
      ...body
    }: {
      owner: string;
      machine_confirm: "none" | "changes" | "all";
      allow_single_user_saved_logins: boolean;
      acknowledge_single_user_risk: boolean;
    }) =>
      apiClient(`/nodes/${encodeURIComponent(id)}/machine-settings`, {
        method: "PUT",
        body,
        ...remoteMachineAuthority(owner),
      }),
    onSuccess: (_response, { owner }) => {
      if (useAuthStore.getState().user?.id === owner)
        return query.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}

export interface MachineDesktopMetadata {
  readonly display?: "secure" | "dev";
  readonly node_id: string;
  readonly session_id: string;
  readonly conversation_id: string | null;
  readonly status: string;
  readonly reason: string | null;
  readonly context_id?: string | null;
}
/**
 * A conversation's machine desktops. Agents open them during a turn, so an
 * idle thread with none polls nothing; the list is re-read when a turn ends.
 */
export function useMachineDesktops(conversation: string, turnActive = false) {
  const queryClient = useQueryClient();
  const queryKey = ["machine-desktops", conversation];
  const wasActive = useRef(turnActive);
  useEffect(() => {
    if (wasActive.current && !turnActive) {
      void queryClient.invalidateQueries({ queryKey, exact: true });
    }
    wasActive.current = turnActive;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [turnActive, conversation]);
  return useQuery({
    queryKey,
    queryFn: () =>
      api.get<MachineDesktopMetadata[]>(
        `/assistant/nyxagent/machines?${new URLSearchParams({ conversation_id: conversation })}`,
      ),
    enabled: Boolean(conversation),
    refetchInterval: (query) =>
      turnActive || query.state.data?.length ? 5000 : false,
  });
}

/** One entry per open desktop (machine, context and display) in a conversation. */
export function useConversationDesktops(conversation: string, turnActive = false) {
  const machines = useMachineDesktops(conversation, turnActive);
  return [
    ...new Map(
      (machines.data ?? []).map((machine) => [conversationDesktopKey(machine), machine]),
    ).values(),
  ];
}

export function conversationDesktopKey(machine: MachineDesktopMetadata): string {
  return `${machine.node_id}:${machine.context_id ?? "legacy"}:${machine.display ?? "secure"}`;
}

export function useMachineUpdates() {
  return useQuery({
    queryKey: ["machine-updates"],
    queryFn: () => api.get<MachineUpdateStatus[]>("/machines/updates"),
    refetchInterval: 3000,
  });
}
export function useStartMachineUpdate(id: string) {
  const query = useQueryClient();
  return useMutation({
    mutationFn: () =>
      api.post<{ status: string; reason?: string }>(
        `/machines/${id}/update`,
        {},
      ),
    onSuccess: () => query.invalidateQueries({ queryKey: ["machine-updates"] }),
  });
}
export function useMachineUpdatePolicy(id: string) {
  const query = useQueryClient();
  return useMutation({
    mutationFn: (automatic: boolean) =>
      api.put(`/machines/${id}/update-policy`, { automatic }),
    onSuccess: () => query.invalidateQueries({ queryKey: ["machine-updates"] }),
  });
}

export function useVerifiedUpdaterImage() {
  return useQuery({
    queryKey: ["machine-updater-image"],
    queryFn: () =>
      api.get<{ version: string; image: string | null; status: string }>(
        "/machines/updater-image",
      ),
    refetchInterval: (query) => (query.state.data?.image ? 3_600_000 : 5000),
  });
}
