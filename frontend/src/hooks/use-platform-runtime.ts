import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { personalActorGuard } from "@/lib/platform-personal";
import { canAdminWrite } from "@/types/api";
import {
  managerRuntimeSchema,
  publicProfilesSchema,
  publicReadinessSchema,
} from "@/schemas/platform-runtime";
import { z } from "zod";

export function useManagerRuntime(id: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["platform-runtime", user?.id, id],
    enabled: canAdminWrite(user),
    retry: false,
    gcTime: 0,
    queryFn: async () =>
      managerRuntimeSchema.parse(
        await apiClient(`/admin/platform-bots/${id}/runtime`, {
          authorityGuard: personalActorGuard(user?.id),
        }),
      ),
  });
}
export function useManagerRuntimeAction(id: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (input: {
      action: "profiles" | "readiness" | "update" | "clear-emergency-stop";
      body: object;
    }) => {
      const path = `/admin/platform-bots/${id}`;
      const endpoint =
        input.action === "profiles" || input.action === "readiness"
          ? `${path}/public/${input.action}`
          : `${path}/runtime${input.action === "clear-emergency-stop" ? "/clear-emergency-stop" : ""}`;
      const result = await apiClient(endpoint, {
        method: input.action === "update" ? "PUT" : "POST",
        body: input.body,
        authorityGuard: personalActorGuard(actor),
      });
      if (input.action === "profiles")
        return publicProfilesSchema.parse(result);
      if (input.action === "readiness")
        return z
          .object({ public_readiness: publicReadinessSchema })
          .parse(result);
      return managerRuntimeSchema.parse(result);
    },
    onSuccess: (result) => {
      if ("runtime" in result)
        client.setQueryData(["platform-runtime", actor, id], result);
    },
  });
}
