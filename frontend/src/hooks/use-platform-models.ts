import { certifiedContractSchema } from "@/schemas/model-contracts";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite, hasAdminRead } from "@/types/api";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  accessExplanationSchema,
  adminModelListSchema,
  adminModelSchema,
  publicModelListSchema,
  tariffSchema,
} from "@/schemas/platform-models";

const root = ["platform-models"] as const;
export const adminModelPath = (id: string) =>
  `/admin/platform-models/${encodeURIComponent(id)}`;
export function usePlatformModels(admin = false) {
  const user = useAuthStore((s) => s.user);
  const enabled = useFeature(FEATURE_FLAG.INFERENCE_PLATFORM_MODELS);
  return useInfiniteQuery({
    queryKey: [...root, admin ? "admin" : "public", user?.id],
    initialPageParam: "",
    queryFn: async ({ pageParam }) => {
      const path = admin ? "/admin/platform-models" : "/llm/models";
      const raw = await api.get(
        `${path}?limit=50${pageParam ? `&after=${encodeURIComponent(pageParam)}` : ""}`,
      );
      return admin
        ? adminModelListSchema.parse(raw)
        : publicModelListSchema.parse(raw);
    },
    getNextPageParam: (page) => page.next_after ?? undefined,
    enabled: !!user && (admin ? hasAdminRead(user) : enabled),
    refetchOnWindowFocus: false,
  });
}
export function useAdminPlatformModel(id: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: [...root, "admin", user?.id, id],
    queryFn: async () =>
      adminModelSchema.parse(await api.get(adminModelPath(id))),
    enabled: !!id && hasAdminRead(user),
    refetchOnWindowFocus: false,
  });
}
export function useModelTariffs(id: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: [...root, "tariffs", user?.id, id],
    queryFn: async () =>
      tariffSchema
        .array()
        .parse(await api.get(`${adminModelPath(id)}/tariffs`)),
    enabled: !!id && hasAdminRead(user),
    refetchOnWindowFocus: false,
  });
}
export function useModelAccess(id: string, person: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: [...root, "access", user?.id, id, person],
    queryFn: async () =>
      accessExplanationSchema.parse(
        await api.get(
          `${adminModelPath(id)}/access?user_id=${encodeURIComponent(person)}`,
        ),
      ),
    enabled: !!id && !!person && hasAdminRead(user),
    refetchOnWindowFocus: false,
  });
}
type Change = { path: string; body: unknown; method?: "post" | "put" };
export function usePlatformModelChange() {
  const client = useQueryClient();
  const user = useAuthStore((s) => s.user);
  const enabled = useFeature(FEATURE_FLAG.INFERENCE_PLATFORM_MODELS);
  return useMutation({
    mutationFn: ({ path, body, method = "put" }: Change) => {
      if (!enabled || !canAdminWrite(user))
        throw new Error("Platform models are read-only for this account.");
      return method === "post" ? api.post(path, body) : api.put(path, body);
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: root });
      await client.invalidateQueries({ queryKey: ["admin", "billing"] });
    },
  });
}

export function useModelContracts() {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: [...root, "contracts", user?.id],
    queryFn: async () =>
      certifiedContractSchema
        .array()
        .parse(await api.get("/admin/platform-models/contracts")),
    enabled: hasAdminRead(user),
    refetchOnWindowFocus: false,
  });
}
