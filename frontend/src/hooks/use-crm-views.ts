import { z } from "zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import {
  crmValidationErrorSchema,
  type CrmValidationError,
  type CrmViewEnvelope,
} from "@/lib/crm/views/schema";
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
});

const crmSavedViewsResponseSchema = z.object({
  views: z.array(crmSavedViewSummarySchema),
});

const crmViewValidationResponseSchema = z.object({
  valid: z.boolean(),
  errors: z.array(crmValidationErrorSchema),
});

export type CrmSavedViewSummary = z.infer<typeof crmSavedViewSummarySchema>;
export type CrmSavedView = z.infer<typeof crmSavedViewSchema>;

export interface CrmViewValidationResponse {
  readonly valid: boolean;
  readonly errors: readonly CrmValidationError[];
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

export function useCrmViews(ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useQuery({
    queryKey: crmViewQueryKeys.list(owner),
    queryFn: async () =>
      crmSavedViewsResponseSchema.parse(
        await assistantJson(
          `${CRM_VIEWS_PATH}${ownerSearch(owner, actor)}`,
        ),
      ).views,
    enabled: Boolean(actor && owner),
    retry: false,
  });
}

export function useCrmView(
  viewId: string | undefined,
  ownerId?: string,
) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useQuery({
    queryKey: crmViewQueryKeys.detail(owner, viewId ?? ""),
    queryFn: async () =>
      crmSavedViewSchema.parse(
        await assistantJson(
          `${CRM_VIEWS_PATH}/${encodeURIComponent(viewId ?? "")}${ownerSearch(owner, actor)}`,
        ),
      ),
    enabled: Boolean(actor && owner && viewId),
    retry: false,
  });
}

export function useValidateCrmView(ownerId?: string) {
  const { actor, owner } = useCrmViewOwner(ownerId);
  return useMutation({
    mutationFn: async (
      envelope: CrmViewEnvelope,
    ): Promise<CrmViewValidationResponse> =>
      crmViewValidationResponseSchema.parse(
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
    }: {
      readonly title: string;
      readonly envelope: CrmViewEnvelope;
    }) =>
      crmSavedViewSchema.parse(
        await assistantJson(CRM_VIEWS_PATH, {
          method: "POST",
          body: ownerBody(
            { title, envelope },
            context.owner,
            context.actor,
          ),
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
    }: {
      readonly id: string;
      readonly title: string;
      readonly envelope: CrmViewEnvelope;
      readonly expectedRevision: number;
    }) =>
      crmSavedViewSchema.parse(
        await assistantJson(
          `${CRM_VIEWS_PATH}/${encodeURIComponent(id)}`,
          {
            method: "PUT",
            body: ownerBody(
              {
                title,
                envelope,
                expected_revision: expectedRevision,
              },
              context.owner,
              context.actor,
            ),
          },
        ),
      ),
    onSuccess: (view) => context.invalidate(view.id),
  });
}

export function useDeleteCrmView(ownerId?: string) {
  const context = useCrmViewMutationContext(ownerId);
  return useMutation({
    mutationFn: async (id: string) => {
      await assistantJson(`${CRM_VIEWS_PATH}/${encodeURIComponent(id)}`, {
        method: "DELETE",
        body: ownerBody({}, context.owner, context.actor),
      });
      return id;
    },
    onSuccess: (id) => context.invalidate(id),
  });
}
