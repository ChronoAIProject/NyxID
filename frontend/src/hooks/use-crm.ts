import { useVisibleCrmQuery } from "@/lib/crm/query-visibility";
import { z } from "zod";
import {
  useMutation,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
  type UseQueryOptions,
} from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  crmActivitiesResponseSchema,
  crmAggregateQuerySchema,
  crmAggregateResultSchema,
  crmFieldSchema,
  crmObjectsResponseSchema,
  crmObjectSchema,
  crmRecordResponseSchema,
  crmRelatedResponseSchema,
  crmTableQuerySchema,
  crmTableResultSchema,
  type CrmAggregateQuery,
  type CrmRecord,
  type CrmTableQuery,
  type CrmTableResult,
} from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";

const CRM_PATH = "/assistant/crm";
const CRM_POLL_INTERVAL_MS = 30_000;

export const crmQueryKeys = {
  root: (owner: string | undefined) => ["crm", owner] as const,
  objects: (owner: string | undefined) => ["crm", owner, "objects"] as const,
  object: (owner: string | undefined, key: string) =>
    ["crm", owner, "object", key] as const,
  table: (owner: string | undefined, query: CrmTableQuery) =>
    ["crm", owner, "table", query] as const,
  aggregate: (owner: string | undefined, query: CrmAggregateQuery) =>
    ["crm", owner, "aggregate", query] as const,
  record: (owner: string | undefined, id: string) =>
    ["crm", owner, "record", id] as const,
  activities: (owner: string | undefined, id: string, cursor?: string) =>
    ["crm", owner, "record", id, "activities", cursor] as const,
  related: (owner: string | undefined, id: string) =>
    ["crm", owner, "record", id, "related"] as const,
};

function useCrmOwner(ownerId?: string) {
  const actor = useAuthStore((state) => state.user?.id);
  return { actor, owner: ownerId ?? actor };
}

function ownerSearch(ownerId: string | undefined, actor: string | undefined) {
  return ownerId && ownerId !== actor
    ? `?owner_id=${encodeURIComponent(ownerId)}`
    : "";
}

function ownerBody<T extends object>(
  body: T,
  owner: string | undefined,
  actor: string | undefined,
) {
  return owner && owner !== actor ? { ...body, owner_id: owner } : body;
}

export function createCrmIdempotencyKey() {
  return crypto.randomUUID();
}

function mutationHeaders(idempotencyKey?: string) {
  const key = idempotencyKey ?? createCrmIdempotencyKey();
  if (!/^[\x20-\x7e]{1,128}$/.test(key)) {
    throw new Error("Idempotency-Key must be 1-128 visible ASCII characters.");
  }
  return { "Idempotency-Key": key };
}

export function useCrmObjects(ownerId?: string, enabled = true) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [...crmQueryKeys.objects(owner), "actor", actor],
      queryFn: async () =>
        crmObjectsResponseSchema.parse(
          await assistantJson(
            `${CRM_PATH}/objects${ownerSearch(owner, actor)}`,
          ),
        ).objects,
      enabled: enabled && Boolean(actor && owner),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmObject(
  objectKey: string | undefined,
  ownerId?: string,
  enabled = true,
) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmQueryKeys.object(owner, objectKey ?? ""),
        "actor",
        actor,
      ],
      queryFn: async () =>
        crmObjectSchema.parse(
          await assistantJson(
            `${CRM_PATH}/objects/${encodeURIComponent(objectKey ?? "")}${ownerSearch(owner, actor)}`,
          ),
        ),
      enabled: enabled && Boolean(actor && owner && objectKey),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmTableQuery(
  query: CrmTableQuery | undefined,
  ownerId?: string,
  options?: Pick<UseQueryOptions<CrmTableResult>, "enabled">,
) {
  const { actor, owner } = useCrmOwner(ownerId);
  const parsed = query ? crmTableQuerySchema.safeParse(query) : undefined;
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmQueryKeys.table(
          owner,
          query ?? { object: "company", filters: [] },
        ),
        "actor",
        actor,
      ],
      queryFn: async () => {
        if (!parsed?.success)
          throw parsed?.error ?? new Error("CRM query is required.");
        return crmTableResultSchema.parse(
          await assistantJson(`${CRM_PATH}/query`, {
            method: "POST",
            body: ownerBody(parsed.data, owner, actor),
          }),
        );
      },
      enabled: (options?.enabled ?? true) && Boolean(actor && owner && query),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmAggregate(
  query: CrmAggregateQuery | undefined,
  ownerId?: string,
  enabled = true,
) {
  const { actor, owner } = useCrmOwner(ownerId);
  const parsed = query ? crmAggregateQuerySchema.safeParse(query) : undefined;
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmQueryKeys.aggregate(
          owner,
          query ?? {
            object: "company",
            filters: [],
            measure: { op: "count" },
            order: "value_desc",
          },
        ),
        "actor",
        actor,
      ],
      queryFn: async () => {
        if (!parsed?.success)
          throw parsed?.error ?? new Error("CRM aggregate is required.");
        return crmAggregateResultSchema.parse(
          await assistantJson(`${CRM_PATH}/aggregate`, {
            method: "POST",
            body: ownerBody(parsed.data, owner, actor),
          }),
        );
      },
      enabled: enabled && Boolean(actor && owner && query && parsed?.success),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmRecord(recordId: string | undefined, ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [...crmQueryKeys.record(owner, recordId ?? ""), "actor", actor],
      queryFn: async () =>
        crmRecordResponseSchema.parse(
          await assistantJson(
            `${CRM_PATH}/records/${encodeURIComponent(recordId ?? "")}${ownerSearch(owner, actor)}`,
          ),
        ),
      enabled: Boolean(actor && owner && recordId),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmActivities(
  recordId: string | undefined,
  cursor?: string,
  ownerId?: string,
) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmQueryKeys.activities(owner, recordId ?? "", cursor),
        "actor",
        actor,
      ],
      queryFn: async () => {
        const params = new URLSearchParams();
        if (cursor) params.set("cursor", cursor);
        if (owner && owner !== actor) params.set("owner_id", owner);
        return crmActivitiesResponseSchema.parse(
          await assistantJson(
            `${CRM_PATH}/records/${encodeURIComponent(recordId ?? "")}/activities${params.size ? `?${params}` : ""}`,
          ),
        );
      },
      enabled: Boolean(actor && owner && recordId),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmActivityHistory(recordId: string, ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useInfiniteQuery({
      queryKey: [
        ...crmQueryKeys.activities(owner, recordId),
        "history",
        "actor",
        actor,
      ],
      initialPageParam: undefined as string | undefined,
      queryFn: async ({ pageParam }) => {
        const params = new URLSearchParams();
        if (pageParam) params.set("cursor", pageParam);
        if (owner && owner !== actor) params.set("owner_id", owner);
        return crmActivitiesResponseSchema.parse(
          await assistantJson(
            `${CRM_PATH}/records/${encodeURIComponent(recordId)}/activities${params.size ? `?${params}` : ""}`,
          ),
        );
      },
      getNextPageParam: (page) => page.next_cursor ?? undefined,
      enabled: Boolean(actor && owner && recordId),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

export function useCrmRelated(recordId: string | undefined, ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmQueryKeys.related(owner, recordId ?? ""),
        "actor",
        actor,
      ],
      queryFn: async () =>
        crmRelatedResponseSchema.parse(
          await assistantJson(
            `${CRM_PATH}/records/${encodeURIComponent(recordId ?? "")}/related${ownerSearch(owner, actor)}`,
          ),
        ),
      enabled: Boolean(actor && owner && recordId),
      retry: false,
      refetchInterval: CRM_POLL_INTERVAL_MS,
      refetchIntervalInBackground: false,
    }),
    owner,
  );
}

function useCrmMutationContext(ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  const client = useQueryClient();
  const invalidate = async (recordId?: string) => {
    await client.invalidateQueries({ queryKey: crmQueryKeys.root(owner) });
    if (recordId) {
      await client.invalidateQueries({
        queryKey: [...crmQueryKeys.record(owner, recordId), "actor", actor],
      });
    }
  };
  return { actor, owner, client, invalidate };
}

export function useCreateCrmRecord(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      object,
      values,
      protect,
      idempotencyKey,
    }: {
      object: string;
      values: Readonly<Record<string, unknown>>;
      protect?: readonly string[];
      idempotencyKey?: string;
    }) =>
      crmRecordSchemaFromMutation(
        await assistantJson(`${CRM_PATH}/records`, {
          method: "POST",
          headers: mutationHeaders(idempotencyKey),
          body: ownerBody(
            { object, values, protect },
            context.owner,
            context.actor,
          ),
        }),
      ),
    onSuccess: (record) => context.invalidate(record.id),
  });
}

function crmRecordSchemaFromMutation(value: unknown): CrmRecord {
  return crmRecordResponseSchema.parse(value);
}

export function useUpdateCrmRecord(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      expectedRevision,
      values,
      protect,
      unprotect,
      idempotencyKey,
    }: {
      id: string;
      expectedRevision: number;
      values: Readonly<Record<string, unknown>>;
      protect?: readonly string[];
      unprotect?: readonly string[];
      idempotencyKey?: string;
    }) =>
      crmRecordSchemaFromMutation(
        await assistantJson(`${CRM_PATH}/records/${encodeURIComponent(id)}`, {
          method: "PATCH",
          headers: mutationHeaders(idempotencyKey),
          body: ownerBody(
            {
              expected_revision: expectedRevision,
              values,
              protect,
              unprotect,
            },
            context.owner,
            context.actor,
          ),
        }),
      ),
    onSuccess: (record) => context.invalidate(record.id),
  });
}

export function useMoveCrmRecord(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      expectedRevision,
      to,
      idempotencyKey,
    }: {
      id: string;
      expectedRevision: number;
      to: string;
      idempotencyKey?: string;
    }) =>
      crmRecordSchemaFromMutation(
        await assistantJson(
          `${CRM_PATH}/records/${encodeURIComponent(id)}/move`,
          {
            method: "POST",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody(
              { expected_revision: expectedRevision, to },
              context.owner,
              context.actor,
            ),
          },
        ),
      ),
    onSuccess: (record) => context.invalidate(record.id),
  });
}

export function useSetCrmRecordArchived(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      expectedRevision,
      archived,
      idempotencyKey,
    }: {
      id: string;
      expectedRevision: number;
      archived: boolean;
      idempotencyKey?: string;
    }) =>
      crmRecordSchemaFromMutation(
        await assistantJson(
          `${CRM_PATH}/records/${encodeURIComponent(id)}/${archived ? "archive" : "restore"}`,
          {
            method: "POST",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody(
              { expected_revision: expectedRevision },
              context.owner,
              context.actor,
            ),
          },
        ),
      ),
    onSuccess: (record) => context.invalidate(record.id),
  });
}

export function useAddCrmNote(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      body,
      idempotencyKey,
    }: {
      id: string;
      body: string;
      idempotencyKey?: string;
    }) =>
      crmActivitiesResponseSchema.shape.activities.element.parse(
        await assistantJson(
          `${CRM_PATH}/records/${encodeURIComponent(id)}/notes`,
          {
            method: "POST",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody({ body }, context.owner, context.actor),
          },
        ),
      ),
    onSuccess: (activity) => context.invalidate(activity.record_id),
  });
}

export function useAddCrmField(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      objectKey,
      expectedSchemaRevision,
      field,
      idempotencyKey,
    }: {
      objectKey: string;
      expectedSchemaRevision: number;
      field: unknown;
      idempotencyKey?: string;
    }) =>
      crmObjectSchema.parse(
        await assistantJson(
          `${CRM_PATH}/objects/${encodeURIComponent(objectKey)}/fields`,
          {
            method: "POST",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody(
              {
                expected_schema_revision: expectedSchemaRevision,
                ...crmFieldSchema.parse(field),
              },
              context.owner,
              context.actor,
            ),
          },
        ),
      ),
    onSuccess: () => context.invalidate(),
  });
}

export function useUpdateCrmField(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      objectKey,
      fieldKey,
      expectedSchemaRevision,
      change,
      idempotencyKey,
    }: {
      objectKey: string;
      fieldKey: string;
      expectedSchemaRevision: number;
      change: Readonly<Record<string, unknown>>;
      idempotencyKey?: string;
    }) =>
      crmObjectSchema.parse(
        await assistantJson(
          `${CRM_PATH}/objects/${encodeURIComponent(objectKey)}/fields/${encodeURIComponent(fieldKey)}`,
          {
            method: "PATCH",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody(
              {
                expected_schema_revision: expectedSchemaRevision,
                ...change,
              },
              context.owner,
              context.actor,
            ),
          },
        ),
      ),
    onSuccess: () => context.invalidate(),
  });
}

const crmPersonSchema = z.object({
  id: z.string(),
  label: z.string(),
  available: z.boolean(),
});
const crmPeopleSchema = z.object({
  people: z.array(crmPersonSchema),
  next_cursor: z.string().nullable(),
});
export function useCrmPeople(
  ownerId?: string,
  cursor?: string,
  enabled = true,
) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: ["crm", owner, "people", cursor, "actor", actor],
      queryFn: async () =>
        crmPeopleSchema.parse(
          await assistantJson(
            `${CRM_PATH}/people?${new URLSearchParams({ ...(owner ? { owner_id: owner } : {}), ...(cursor ? { cursor } : {}) })}`,
          ),
        ),
      enabled: enabled && Boolean(actor && owner),
      retry: false,
    }),
    owner,
  );
}
export function useCrmPerson(id: string | undefined, ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: ["crm", owner, "person", id, "actor", actor],
      queryFn: async () =>
        crmPersonSchema.parse(
          await assistantJson(
            `${CRM_PATH}/people/${encodeURIComponent(id ?? "")}${ownerSearch(owner, actor)}`,
          ),
        ),
      enabled: Boolean(actor && owner && id),
      retry: false,
    }),
    owner,
  );
}

const importSnapshotSchema = z.object({
  schema_revision: z.number().int(),
  payload_digest: z.string(),
  rows: z.array(
    z.object({
      index: z.number().int(),
      matched_id: z.string().nullable(),
      revision: z.number().int().nullable(),
      action: z.string(),
    }),
  ),
});
const importResultSchema = z.object({
  committed: z.boolean(),
  snapshot: importSnapshotSchema,
  rows: z.array(
    z.object({
      index: z.number().int(),
      matched_id: z.string().nullable(),
      revision: z.number().int().nullable(),
      action: z.string(),
      errors: z.array(z.object({ path: z.string(), message: z.string() })),
    }),
  ),
});
export type CrmImportResult = z.infer<typeof importResultSchema>;
export interface CrmImportInput {
  object: string;
  mapping: Record<string, string>;
  rows: Record<string, unknown>[];
  match_field: string;
  policy: "create_only" | "update_or_create" | "skip_existing";
  snapshot?: CrmImportResult["snapshot"];
}
export function useCrmImport(ownerId?: string) {
  const context = useCrmMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      input,
      commit,
      idempotencyKey,
    }: {
      input: CrmImportInput;
      commit: boolean;
      idempotencyKey?: string;
    }) =>
      importResultSchema.parse(
        await assistantJson(
          `${CRM_PATH}/import/${commit ? "commit" : "preview"}`,
          {
            method: "POST",
            headers: mutationHeaders(idempotencyKey),
            body: ownerBody(input, context.owner, context.actor),
          },
        ),
      ),
    onSuccess: (result) =>
      result.committed ? context.invalidate() : undefined,
  });
}
export function useCrmExport(ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  return useMutation({
    mutationFn: async (query: CrmTableQuery) =>
      crmTableResultSchema.parse(
        await assistantJson(`${CRM_PATH}/export`, {
          method: "POST",
          body: ownerBody(query, owner, actor),
        }),
      ),
  });
}

export function useCrmPeopleLabels(ids: readonly string[], ownerId?: string) {
  const { actor, owner } = useCrmOwner(ownerId);
  const unique = [...new Set(ids)].sort();
  return useVisibleCrmQuery(
    useQuery({
      queryKey: ["crm", owner, "people-labels", unique, "actor", actor],
      queryFn: async () => {
        const people: z.infer<typeof crmPersonSchema>[] = [];
        for (let index = 0; index < unique.length; index += 200) {
          const result = z
            .object({ people: z.array(crmPersonSchema) })
            .parse(
              await assistantJson(`${CRM_PATH}/people/lookup`, {
                method: "POST",
                body: ownerBody(
                  { ids: unique.slice(index, index + 200) },
                  owner,
                  actor,
                ),
              }),
            );
          people.push(...result.people);
        }
        return people;
      },
      enabled: Boolean(actor && owner && unique.length),
      retry: false,
      staleTime: 30_000,
    }),
    owner,
  );
}
