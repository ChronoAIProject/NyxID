import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api-client";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type {
  FeatureFlagHistoryResponse,
  AdminFeatureFlagListResponse,
  CreateFeatureFlagRequest,
  FeatureFlagTarget,
  AdminFeatureFlag,
  AdminFeatureFlagTargetKind,
  SetAdminFeatureFlagRequest,
  UpdateAdminFeatureFlagMetadataRequest,
} from "@/schemas/admin-feature-flags";

const QUERY_KEY = ["admin", "feature-flags"] as const;

interface SavedOverride {
  enabled: boolean;
  updated_at: string;
  updated_by: string;
}

function withOverride(
  flag: AdminFeatureFlag,
  kind: AdminFeatureFlagTargetKind,
  target: string | null | undefined,
  saved: SavedOverride | null,
): AdminFeatureFlag {
  if (kind === "global")
    return { ...flag, global_override: saved?.enabled ?? null };
  if (!target) return flag;
  if (kind === "org") {
    const previous = flag.org_overrides.find((row) => row.org_id === target);
    return {
      ...flag,
      org_overrides: [
        ...flag.org_overrides.filter((row) => row.org_id !== target),
        ...(saved
          ? [
              {
                org_id: target,
                org_display_name: null,
                org_slug: null,
                ...previous,
                ...saved,
              },
            ]
          : []),
      ],
    };
  }
  const previous = flag.user_overrides.find((row) => row.user_id === target);
  return {
    ...flag,
    user_overrides: [
      ...flag.user_overrides.filter((row) => row.user_id !== target),
      ...(saved
        ? [
            {
              user_id: target,
              user_email: null,
              user_display_name: null,
              ...previous,
              ...saved,
            },
          ]
        : []),
    ],
  };
}

export function useAdminFeatureFlags() {
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: (): Promise<AdminFeatureFlagListResponse> =>
      api.get<AdminFeatureFlagListResponse>("/admin/feature-flags"),
  });
}

export function useSetAdminFeatureFlag() {
  const queryClient = useQueryClient();
  return useMutation({
    onMutate: () => queryClient.cancelQueries({ queryKey: QUERY_KEY }),
    mutationFn: ({
      flagKey,
      body,
    }: {
      flagKey: string;
      body: SetAdminFeatureFlagRequest;
    }): Promise<SavedOverride> =>
      api.put(`/admin/feature-flags/${encodeURIComponent(flagKey)}`, body),
    onSuccess: async (saved, { flagKey, body }) => {
      await queryClient.cancelQueries({ queryKey: QUERY_KEY });
      queryClient.setQueryData<AdminFeatureFlagListResponse>(
        QUERY_KEY,
        (current) =>
          current && {
            ...current,
            flags: current.flags.map((flag) =>
              flag.key === flagKey
                ? withOverride(flag, body.target_kind, body.target_key, saved)
                : flag,
            ),
          },
      );
      void queryClient.invalidateQueries({
        queryKey: ["admin", "feature-flag-history"],
      });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: ["orgs"] });
      void queryClient.invalidateQueries({ queryKey: ["user"] });
      if (flagKey === FEATURE_FLAG.INVITATION_CODE)
        void queryClient.invalidateQueries({ queryKey: ["public-config"] });
    },
  });
}

/**
 * Edit documentation, organization and lifecycle without changing rollout.
 */
export function useUpdateAdminFeatureFlagMetadata() {
  const queryClient = useQueryClient();
  return useMutation({
    onMutate: () => queryClient.cancelQueries({ queryKey: QUERY_KEY }),
    mutationFn: ({
      flagKey,
      body,
    }: {
      flagKey: string;
      body: UpdateAdminFeatureFlagMetadataRequest;
    }) =>
      api.patch<
        Pick<
          AdminFeatureFlag,
          | "key"
          | "description"
          | "code_description"
          | "custom_description"
          | "owner"
          | "group"
          | "tags"
          | "sunset"
          | "metadata_updated_at"
          | "metadata_updated_by"
        >
      >(`/admin/feature-flags/${encodeURIComponent(flagKey)}/metadata`, body),
    onSuccess: async (saved) => {
      await queryClient.cancelQueries({ queryKey: QUERY_KEY });
      queryClient.setQueryData<AdminFeatureFlagListResponse>(
        QUERY_KEY,
        (current) =>
          current && {
            ...current,
            flags: current.flags.map((flag) =>
              flag.key === saved.key ? { ...flag, ...saved } : flag,
            ),
          },
      );
      void queryClient.invalidateQueries({
        queryKey: ["admin", "feature-flag-history"],
      });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
    onError: async (error) => {
      if (error instanceof ApiError && error.status === 409)
        await queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
  });
}

export function useClearAdminFeatureFlag() {
  const queryClient = useQueryClient();
  return useMutation({
    onMutate: () => queryClient.cancelQueries({ queryKey: QUERY_KEY }),
    mutationFn: ({
      flagKey,
      targetKind,
      targetKey,
    }: {
      flagKey: string;
      targetKind: AdminFeatureFlagTargetKind;
      targetKey?: string | null;
    }): Promise<void> => {
      const params = new URLSearchParams({ target_kind: targetKind });
      if (targetKey != null) params.set("target_key", targetKey);
      return api.delete<void>(
        `/admin/feature-flags/${encodeURIComponent(flagKey)}?${params.toString()}`,
      );
    },
    onSuccess: async (_, { flagKey, targetKind, targetKey }) => {
      await queryClient.cancelQueries({ queryKey: QUERY_KEY });
      queryClient.setQueryData<AdminFeatureFlagListResponse>(
        QUERY_KEY,
        (current) =>
          current && {
            ...current,
            flags: current.flags.map((flag) =>
              flag.key === flagKey
                ? withOverride(flag, targetKind, targetKey, null)
                : flag,
            ),
          },
      );
      void queryClient.invalidateQueries({
        queryKey: ["admin", "feature-flag-history"],
      });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: ["orgs"] });
      void queryClient.invalidateQueries({ queryKey: ["user"] });
      if (flagKey === FEATURE_FLAG.INVITATION_CODE)
        void queryClient.invalidateQueries({ queryKey: ["public-config"] });
    },
  });
}

export function useCreateFeatureFlag() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateFeatureFlagRequest) =>
      api.post("/admin/feature-flags", body),
    onSuccess: async () => {
      await client.cancelQueries({ queryKey: QUERY_KEY });
      await client.invalidateQueries({ queryKey: QUERY_KEY });
      await client.invalidateQueries({ queryKey: ["user", "me"] });
    },
  });
}

export function useFeatureFlagTargets(
  flagKey: string,
  kind: "person" | "org",
  search?: string,
  limit = 20,
  enabled = true,
) {
  return useQuery({
    queryKey: [...QUERY_KEY, "targets", flagKey, kind, search, limit],
    queryFn: async () => {
      const params = new URLSearchParams({
        flag_key: flagKey,
        kind,
        limit: String(limit),
      });
      if (search) params.set("search", search);
      const users = await api.get<FeatureFlagTarget[]>(
        `/admin/feature-flag-targets?${params}`,
      );
      return { users, total: users.length };
    },
    enabled,
  });
}

export function useFeatureFlagHistory(
  flagKey: string,
  cursor: string | null,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["admin", "feature-flag-history", flagKey, cursor],
    queryFn: () =>
      api.get<FeatureFlagHistoryResponse>(
        `/admin/feature-flags/${encodeURIComponent(flagKey)}/history${cursor ? `?before=${encodeURIComponent(cursor)}` : ""}`,
      ),
    enabled,
  });
}
