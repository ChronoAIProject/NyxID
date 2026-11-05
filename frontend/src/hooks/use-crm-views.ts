import { useVisibleCrmQuery } from "@/lib/crm/query-visibility";
import { z } from "zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  crmValidationErrorSchema,
  type CrmValidationError,
  type CrmViewEnvelope,
} from "@/lib/crm/views/schema";
import { crmActorSchema } from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";

const CRM_VIEWS_PATH = "/assistant/crm/views";

const crmSavedViewSummarySchema = z.object({
  id: z.string().min(1),
  owner_id: z.string().min(1),
  title: z.string().min(1),
  revision: z.number().int().positive(),
  valid: z.boolean().optional().default(true),
  errors: z.array(crmValidationErrorSchema).optional().default([]),
  created_at: z.string().datetime({ offset: true }),
  updated_at: z.string().datetime({ offset: true }),
});

const crmSavedViewSchema = crmSavedViewSummarySchema.extend({
  envelope: z.unknown(),
  created_by: crmActorSchema.optional(),
});

const crmSavedViewsResponseSchema = z.object({
  views: z.array(crmSavedViewSummarySchema),
  next_cursor: z.string().nullish(),
});

const crmViewValidationResponseSchema = z.object({
  valid: z.boolean(),
  errors: z.array(crmValidationErrorSchema),
});

const crmViewDeleteResponseSchema = z.object({
  id: z.string().min(1),
  deleted: z.literal(true),
});

export type CrmSavedViewSummary = z.infer<typeof crmSavedViewSummarySchema>;
export type CrmSavedView = z.infer<typeof crmSavedViewSchema>;

export interface CrmViewValidationResponse {
  readonly valid: boolean;
  readonly errors: readonly CrmValidationError[];
}

export function parseCrmSavedViewsResponse(
  response: unknown,
): CrmSavedViewSummary[] {
  return crmSavedViewsResponseSchema.parse(response).views;
}

export function parseCrmSavedViewResponse(response: unknown): CrmSavedView {
  return crmSavedViewSchema.parse(response);
}

export function parseCrmViewValidationResponse(
  response: unknown,
): CrmViewValidationResponse {
  return crmViewValidationResponseSchema.parse(response);
}

export function parseCrmViewDeleteResponse(response: unknown) {
  return crmViewDeleteResponseSchema.parse(response);
}

export const crmViewQueryKeys = {
  list: (owner: string | undefined) => ["crm", owner, "views"] as const,
  detail: (owner: string | undefined, id: string) =>
    ["crm", owner, "views", id] as const,
};

function useCrmViewOwner(ownerId?: string) {
  const actor = useAuthStore((state) => state.user?.id);
  return { actor, owner: ownerId ?? actor };
}

function ownerSearch(owner: string | undefined, actor: string | undefined) {
  return owner && owner !== actor
    ? `?owner_id=${encodeURIComponent(owner)}`
    : "";
}

function ownerBody<T extends object>(
  body: T,
  owner: string | undefined,
  actor: string | undefined,
) {
  return owner && owner !== actor ? { ...body, owner_id: owner } : body;
}

function mutationHeaders(idempotencyKey?: string) {
  const key = idempotencyKey ?? crypto.randomUUID();
  if (!/^[\x20-\x7e]{1,128}$/.test(key)) {
    throw new Error("Idempotency-Key must be 1-128 visible ASCII characters.");
  }
  return { "Idempotency-Key": key };
}

function useCrmViewMutationContext(ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  const queryClient = useQueryClient();
  const invalidate = async (id?: string) => {
    await queryClient.invalidateQueries({
      queryKey: crmViewQueryKeys.list(owner),
    });
    if (id) {
      await queryClient.invalidateQueries({
        queryKey: crmViewQueryKeys.detail(owner, id),
      });
    }
  };
  return { actor, owner, invalidate };
}

export function useCrmViews(ownerId?: string, cursor?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [...crmViewQueryKeys.list(owner), cursor, "actor", actor],
      queryFn: async () =>
        crmSavedViewsResponseSchema.parse(
          await assistantJson(
            `${CRM_VIEWS_PATH}${ownerSearch(owner, actor)}${cursor ? `${ownerSearch(owner, actor) ? "&" : "?"}cursor=${encodeURIComponent(cursor)}` : ""}`,
          ),
        ),
      enabled: Boolean(actor && owner),
      retry: false,
      refetchInterval: 30_000,
    }),
    owner,
  );
}

export function useCrmView(viewId: string | undefined, ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useVisibleCrmQuery(
    useQuery({
      queryKey: [
        ...crmViewQueryKeys.detail(owner, viewId ?? ""),
        "actor",
        actor,
      ],
      queryFn: async () =>
        parseCrmSavedViewResponse(
          await assistantJson(
            `${CRM_VIEWS_PATH}/${encodeURIComponent(viewId ?? "")}${ownerSearch(owner, actor)}`,
          ),
        ),
      enabled: Boolean(actor && owner && viewId),
      retry: false,
      refetchInterval: 30_000,
    }),
    owner,
  );
}

export function useValidateCrmView(ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useMutation({
    mutationFn: async (
      envelope: CrmViewEnvelope,
    ): Promise<CrmViewValidationResponse> =>
      parseCrmViewValidationResponse(
        await assistantJson(`${CRM_VIEWS_PATH}/validate`, {
          method: "POST",
          body: ownerBody({ envelope }, owner, actor),
        }),
      ),
  });
}

export function useCreateCrmView(ownerId?: string) {
  const context = useCrmViewMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      title,
      envelope,
      idempotencyKey,
    }: {
      readonly title: string;
      readonly envelope: CrmViewEnvelope;
      readonly idempotencyKey?: string;
    }) =>
      parseCrmSavedViewResponse(
        await assistantJson(CRM_VIEWS_PATH, {
          method: "POST",
          headers: mutationHeaders(idempotencyKey),
          body: ownerBody({ title, envelope }, context.owner, context.actor),
        }),
      ),
    onSuccess: (view) => context.invalidate(view.id),
  });
}

export function useUpdateCrmView(ownerId?: string) {
  const context = useCrmViewMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      title,
      envelope,
      expectedRevision,
      idempotencyKey,
    }: {
      readonly id: string;
      readonly title: string;
      readonly envelope: CrmViewEnvelope;
      readonly expectedRevision: number;
      readonly idempotencyKey?: string;
    }) =>
      parseCrmSavedViewResponse(
        await assistantJson(`${CRM_VIEWS_PATH}/${encodeURIComponent(id)}`, {
          method: "PUT",
          headers: mutationHeaders(idempotencyKey),
          body: ownerBody(
            {
              title,
              envelope,
              expected_revision: expectedRevision,
            },
            context.owner,
            context.actor,
          ),
        }),
      ),
    onSuccess: (view) => context.invalidate(view.id),
  });
}

export function useDeleteCrmView(ownerId?: string) {
  const context = useCrmViewMutationContext(ownerId);
  return useMutation({
    mutationFn: async ({
      id,
      idempotencyKey,
    }: {
      readonly id: string;
      readonly idempotencyKey?: string;
    }) => {
      const response = parseCrmViewDeleteResponse(
        await assistantJson(`${CRM_VIEWS_PATH}/${encodeURIComponent(id)}`, {
          method: "DELETE",
          headers: mutationHeaders(idempotencyKey),
          body: ownerBody({}, context.owner, context.actor),
        }),
      );
      return response.id;
    },
    onSuccess: (id) => context.invalidate(id),
  });
}

const crmGenerationSchema = z.object({
  envelope: z.unknown(),
  valid: z.boolean(),
  errors: z.array(crmValidationErrorSchema),
});
export function useGenerateCrmView(ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useMutation({
    mutationFn: async (input: {
      object: string;
      prompt: string;
      draft?: unknown;
      idempotencyKey: string;
    }) =>
      crmGenerationSchema.parse(
        await assistantJson(`${CRM_VIEWS_PATH}/generate`, {
          method: "POST",
          headers: mutationHeaders(input.idempotencyKey),
          body: ownerBody(
            { object: input.object, prompt: input.prompt, draft: input.draft },
            owner,
            actor,
          ),
        }),
      ),
  });
}
