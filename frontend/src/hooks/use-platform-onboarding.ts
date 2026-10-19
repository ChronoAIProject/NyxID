import { orgListResponseSchema } from "@/schemas/orgs";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  gatewayCleanupTaskSchema,
  gatewayCleanupReviewSchema,
  type GatewayCleanupReview,
  bindingPreviewSchema,
  bindingSchema,
  platformProfileSchema,
  platformProfilesSchema,
  setupSchema,
  setupApprovalSchema,
  type SetupApproval,
  templatesSchema,
  type PlatformProfile,
  type PlatformSetup,
  type SetupFacts,
  type SetupTemplate,
} from "@/schemas/platform-onboarding";

function actorGuard(actor: string | undefined, admin = false) {
  return () => {
    const user = useAuthStore.getState().user;
    if (!actor || user?.id !== actor || (admin && !canAdminWrite(user)))
      throw new Error(
        "Your account or permissions changed. Reload before continuing.",
      );
  };
}
export function useOnboardingTemplates() {
  return useQuery({
    queryKey: ["onboarding", "templates"],
    queryFn: async () =>
      templatesSchema.parse(await apiClient("/onboarding/templates")),
  });
}
export function usePlatformSetups() {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["onboarding", actor, "setups"],
    enabled: !!actor,
    gcTime: 0,
    queryFn: async () =>
      z.array(setupSchema).parse(
        await apiClient("/onboarding/setups", {
          authorityGuard: actorGuard(actor),
        }),
      ),
  });
}
export function useArchivedSetups(cursor?: { ms: number; id: string }) {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["onboarding", actor, "archives", cursor],
    enabled: !!actor,
    gcTime: 0,
    queryFn: async () =>
      z
        .object({
          setups: z.array(setupSchema),
          next_before_ms: z.number().nullable(),
          next_before_id: z.string().nullable(),
        })
        .parse(
          await apiClient(
            `/onboarding/setups/archived${cursor ? `?before_ms=${cursor.ms}&before_id=${cursor.id}` : ""}`,
            { authorityGuard: actorGuard(actor) },
          ),
        ),
  });
}

export function usePlatformSetup(id: string) {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["onboarding", actor, "setup", id],
    refetchInterval: (query) =>
      query.state.dataUpdateCount + query.state.errorUpdateCount < 120 &&
      [
        "provisioning",
        "awaiting_child_review",
        "ready_for_test",
        "failed_retryable",
      ].includes(query.state.data?.state ?? "")
        ? 5000
        : false,
    enabled: !!actor && z.uuid().safeParse(id).success,
    gcTime: 0,
    retry: false,
    queryFn: async () =>
      setupSchema.parse(
        await apiClient(`/onboarding/setups/${id}`, {
          authorityGuard: actorGuard(actor),
        }),
      ),
  });
}
export function useSetupMutation(id?: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    mutationFn: async (
      input:
        | {
            action: "create";
            request_id: string;
            template: SetupTemplate;
            facts: SetupFacts;
          }
        | {
            action: "edit";
            revision: number;
            facts: SetupFacts;
            limits: PlatformSetup["limits"];
          }
        | {
            action: "binding";
            revision: number;
            binding_id: string;
            binding_generation: number;
          }
        | {
            action: "review" | "publish" | "pause" | "cancel";
            revision: number;
          },
    ) => {
      const { action, ...body } = input;
      const path =
        action === "create"
          ? "/onboarding/setups"
          : `/onboarding/setups/${id}${["edit", "cancel"].includes(action) ? "" : `/${action}`}`;
      const method =
        action === "edit" ? "PUT" : action === "cancel" ? "DELETE" : "POST";
      return setupSchema.parse(
        await apiClient(path, {
          method,
          body,
          authorityGuard: actorGuard(actor),
        }),
      );
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["onboarding", actor] });
    },
  });
}
export function useOwnedBindings() {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["onboarding", actor, "bindings"],
    enabled: !!actor,
    gcTime: 0,
    queryFn: async () =>
      z.array(bindingSchema).parse(
        await apiClient("/onboarding/bindings", {
          authorityGuard: actorGuard(actor),
        }),
      ),
  });
}
export function useRevokeBinding() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: (binding: z.infer<typeof bindingSchema>) =>
      apiClient(`/onboarding/bindings/${binding.id}/revoke`, {
        method: "POST",
        body: { generation: binding.generation },
        authorityGuard: actorGuard(actor),
      }),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["onboarding", actor] }),
  });
}
export function useBindingPreview(code: string) {
  return useQuery({
    queryKey: ["onboarding-link-preview", code],
    enabled: !!code,
    retry: false,
    gcTime: 0,
    staleTime: 0,
    queryFn: async () =>
      bindingPreviewSchema.parse(
        await apiClient("/onboarding/link/preview", {
          method: "POST",
          body: { code },
        }),
      ),
  });
}
export function useBindingDecision(code: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      approve: boolean;
      matched_code: string;
      replace_existing: boolean;
    }) =>
      bindingPreviewSchema.parse(
        await apiClient(
          `/onboarding/link/${input.approve ? "approve" : "deny"}`,
          {
            method: "POST",
            body: {
              code,
              matched_code: input.matched_code,
              replace_existing: input.replace_existing,
            },
            authorityGuard: actorGuard(actor),
          },
        ),
      ),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["onboarding-link-preview", code] }),
  });
}
export function useAdminPlatformProfiles(before?: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["admin-platform-profiles", user?.id, before],
    enabled: canAdminWrite(user),
    gcTime: 0,
    queryFn: async () =>
      platformProfilesSchema.parse(
        await apiClient(
          `/admin/platform-bots${before ? `?before_id=${before}` : ""}`,
          { authorityGuard: actorGuard(user?.id, true) },
        ),
      ),
  });
}
export function useAdminPlatformProfile(profile?: PlatformProfile) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["admin-platform-profile", user?.id, profile?.id],
    enabled: !!profile && canAdminWrite(user),
    gcTime: 0,
    initialData: profile,
    queryFn: async () =>
      platformProfileSchema.parse(
        await apiClient(`/admin/platform-bots/${profile!.id}`, {
          authorityGuard: actorGuard(user?.id, true),
        }),
      ),
  });
}
export function useAdminPlatformAction() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    mutationFn: async (
      input:
        | {
            action: "convert";
            channel_bot_id: string;
            funding_owner_user_id?: string;
          }
        | {
            action: "pause" | "verify" | "repair" | "retire";
            profile: PlatformProfile;
          },
    ) => {
      const path =
        input.action === "convert"
          ? "/admin/platform-bots"
          : `/admin/platform-bots/${input.profile.id}${input.action === "retire" ? "" : `/${input.action}`}`;
      const body =
        input.action === "convert"
          ? {
              channel_bot_id: input.channel_bot_id,
              funding_owner_user_id: input.funding_owner_user_id,
            }
          : { generation: input.profile.generation };
      return platformProfileSchema.parse(
        await apiClient(path, {
          method: input.action === "retire" ? "DELETE" : "POST",
          body,
          authorityGuard: actorGuard(actor, true),
        }),
      );
    },
    onSuccess: () => {
      for (const queryKey of [
        ["admin-platform-profiles"],
        ["admin-platform-profile"],
        ["admin-platform-targets"],
        ["platform-runtime"],
      ])
        void client.invalidateQueries({ queryKey });
    },
  });
}

export function useAdminPlatformTargets(includeOrganizations = false) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["admin-platform-targets", user?.id, includeOrganizations],
    enabled: canAdminWrite(user),
    gcTime: 0,
    queryFn: async () => {
      const guard = actorGuard(user?.id, true);
      const [bots, orgs] = await Promise.all([
        (async () => {
          const bots: {
            id: string;
            platform: string;
            label: string;
            platform_bot_username: string;
            credential_source: string;
          }[] = [];
          let before: string | undefined;
          do {
            const page = await apiClient<{
              bots: typeof bots;
              next_before_id: string | null;
            }>(
              `/admin/platform-bots/targets${before ? `?before_id=${before}` : ""}`,
              { authorityGuard: guard },
            );
            bots.push(...page.bots);
            before = page.next_before_id ?? undefined;
          } while (before);
          return { bots };
        })(),
        includeOrganizations
          ? apiClient("/orgs", { authorityGuard: guard })
          : Promise.resolve({ orgs: [] }),
      ]);
      return {
        bots: bots.bots,
        orgs: orgListResponseSchema
          .parse(orgs)
          .orgs.filter(
            (org) => org.your_role === "admin" || org.your_role === "owner",
          ),
      };
    },
  });
}

export function useSetupApproval(id: string, enabled: boolean) {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["onboarding", actor, "approval", id],
    refetchInterval: (query) =>
      query.state.dataUpdateCount + query.state.errorUpdateCount < 120
        ? 5000
        : false,
    enabled: !!actor && enabled && z.uuid().safeParse(id).success,
    gcTime: 0,
    retry: false,
    queryFn: async () =>
      setupApprovalSchema.parse(
        await apiClient(`/onboarding/setups/${id}/approval`, {
          authorityGuard: actorGuard(actor),
        }),
      ),
  });
}

export function useSetupProvisioning(id: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (
      input:
        | { action: "approve" | "child-approve"; review: SetupApproval }
        | { action: "activate"; revision: number }
        | { action: "launch" },
    ) => {
      const path =
        input.action === "launch"
          ? "telegram-launch"
          : input.action === "approve"
            ? "approval"
            : input.action === "child-approve"
              ? "child-approval"
              : "activate";
      const body =
        input.action === "launch"
          ? {}
          : input.action === "activate"
            ? { revision: input.revision }
            : {
                revision: input.review.reviewed_revision,
                digest: input.review.digest,
                ...(input.action === "child-approve"
                  ? {
                      request_revision: input.review.request_revision,
                      child_bot_id: input.review.child_bot_id,
                      manager_revision: input.review.manager_revision,
                    }
                  : {}),
              };
      const result = await apiClient(`/onboarding/setups/${id}/${path}`, {
        method: "POST",
        body,
        authorityGuard: actorGuard(actor),
      });
      return input.action === "launch"
        ? z.object({ url: z.url() }).parse(result)
        : setupSchema.parse(result);
    },
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["onboarding", actor] }),
  });
}

export function useAdminGuideMutation() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (
      input:
        | {
            action: "discover";
            profile: PlatformProfile;
            model_service_id: string;
          }
        | {
            action: "configure";
            profile: PlatformProfile;
            guide: NonNullable<PlatformProfile["guide"]>;
            enabled: boolean;
          },
    ) => {
      const path = `/admin/platform-bots/${input.profile.id}/guide`;
      if (input.action === "discover")
        return z
          .object({
            generation: z.number().int(),
            model_service_id: z.uuid(),
            models: z.array(z.string()),
          })
          .parse(
            await apiClient(`${path}/models`, {
              method: "POST",
              body: {
                generation: input.profile.generation,
                model_service_id: input.model_service_id,
              },
              authorityGuard: actorGuard(actor, true),
            }),
          );
      return platformProfileSchema.parse(
        await apiClient(path, {
          method: "PUT",
          body: {
            generation: input.profile.generation,
            guide: input.guide,
            enabled: input.enabled,
          },
          authorityGuard: actorGuard(actor, true),
        }),
      );
    },
    onSuccess: () =>
      client.invalidateQueries({
        queryKey: ["admin-platform-profiles", actor],
      }),
  });
}

export function useAnonymousDrafts(binding?: {
  id: string;
  generation: number;
}) {
  const actor = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: [
      "onboarding",
      actor,
      "anonymous",
      binding?.id,
      binding?.generation,
    ],
    enabled: !!actor && !!binding,
    gcTime: 0,
    retry: false,
    queryFn: async () =>
      z.array(setupSchema).parse(
        await apiClient(
          `/onboarding/anonymous-drafts?binding_id=${binding!.id}&binding_generation=${binding!.generation}`,
          {
            authorityGuard: actorGuard(actor),
          },
        ),
      ),
  });
}

export function useAdoptAnonymousDraft(binding: {
  id: string;
  generation: number;
}) {
  const actor = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (draft: PlatformSetup) =>
      setupSchema.parse(
        await apiClient(`/onboarding/setups/${draft.id}/adopt`, {
          method: "POST",
          body: {
            revision: draft.revision,
            binding_id: binding.id,
            binding_generation: binding.generation,
          },
          authorityGuard: actorGuard(actor),
        }),
      ),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["onboarding", actor] }),
  });
}

export function useAdminGuideServices(
  profile: PlatformProfile,
  after?: string,
) {
  const actor = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: [
      "admin-platform-guide-services",
      actor,
      profile.id,
      profile.generation,
      after,
    ],
    enabled: !!actor && !profile.deleted_at,
    gcTime: 0,
    queryFn: async () =>
      z
        .object({
          services: z.array(z.object({ id: z.uuid(), name: z.string() })),
          next_after_id: z.uuid().nullable(),
        })
        .parse(
          await apiClient(
            `/admin/platform-bots/${profile.id}/guide/services${after ? `?after_id=${after}` : ""}`,
            {
              authorityGuard: actorGuard(actor, true),
            },
          ),
        ),
  });
}

export function useGatewayCleanup(profile: PlatformProfile) {
  const actor = useAuthStore((s) => s.user?.id);
  const admin = useAuthStore((s) => canAdminWrite(s.user));
  return useQuery({
    queryKey: [
      "admin",
      actor,
      "platform-bots",
      profile.id,
      profile.generation,
      "cleanup",
    ],
    enabled: !!actor && admin,
    gcTime: 0,
    queryFn: async () =>
      z.array(gatewayCleanupTaskSchema).parse(
        await apiClient(`/admin/platform-bots/${profile.id}/cleanup`, {
          authorityGuard: actorGuard(actor, true),
        }),
      ),
  });
}
export function useGatewayCleanupAction(profile: PlatformProfile) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (
      input:
        | { task: string; action: "inspect" }
        | {
            task: string;
            action: "retry_original" | "preserve_changed" | "confirm_missing";
            review: GatewayCleanupReview;
          },
    ) => {
      const result = await apiClient(
        `/admin/platform-bots/${profile.id}/cleanup/${input.task}/${input.action === "inspect" ? "inspect" : "resolve"}`,
        {
          method: "POST",
          body:
            input.action === "inspect"
              ? {}
              : {
                  profile_generation: input.review.profile_generation,
                  captured_version: input.review.captured_version,
                  observed_version: input.review.observed_version,
                  action: input.action,
                },
          authorityGuard: actorGuard(actor, true),
        },
      );
      return input.action === "inspect"
        ? gatewayCleanupReviewSchema.parse(result)
        : null;
    },
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["admin", actor, "platform-bots"] }),
  });
}
