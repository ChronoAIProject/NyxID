import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  helpLibrarySchema,
  helpRevisionSchema,
  helpAssignmentSchema,
  type HelpContent,
} from "@/schemas/platform-help";
function guard(actor?: string) {
  return () => {
    const user = useAuthStore.getState().user;
    if (!actor || user?.id !== actor || !canAdminWrite(user))
      throw new Error(
        "Your account or admin access changed. Reload before continuing.",
      );
  };
}
export function useHelpLibraries() {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["platform-help", user?.id],
    enabled: canAdminWrite(user),
    gcTime: 0,
    refetchOnWindowFocus: false,
    queryFn: async () =>
      z.array(helpLibrarySchema).parse(
        await apiClient("/admin/platform-help", {
          authorityGuard: guard(user?.id),
        }),
      ),
  });
}
export function useHelpHistory(id: string, before?: number) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["platform-help", user?.id, id, "history", before],
    enabled: canAdminWrite(user),
    gcTime: 0,
    refetchOnWindowFocus: false,
    queryFn: async () =>
      z
        .array(helpRevisionSchema)
        .parse(
          await apiClient(
            `/admin/platform-help/${id}/history${before ? `?before=${before}` : ""}`,
            { authorityGuard: guard(user?.id) },
          ),
        ),
  });
}
export function useHelpMutation() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: async (
      input:
        | { action: "create"; name: string }
        | { action: "save"; id: string; version: number; content: HelpContent }
        | {
            action: "publish";
            id: string;
            version: number;
            rollback_revision?: number;
          },
    ) => {
      const path =
        input.action === "create"
          ? "/admin/platform-help"
          : `/admin/platform-help/${input.id}${input.action === "publish" ? "/publish" : ""}`;
      const body =
        input.action === "create"
          ? { name: input.name }
          : input.action === "save"
            ? { version: input.version, content: input.content }
            : {
                version: input.version,
                rollback_revision: input.rollback_revision,
              };
      return helpLibrarySchema.parse(
        await apiClient(path, {
          method: input.action === "save" ? "PUT" : "POST",
          body,
          authorityGuard: guard(actor),
        }),
      );
    },
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["platform-help", actor] }),
  });
}
export function useHelpAssignment(profile: string) {
  const user = useAuthStore((s) => s.user);
  return useQuery({
    queryKey: ["platform-help-assignment", user?.id, profile],
    enabled: canAdminWrite(user),
    gcTime: 0,
    refetchOnWindowFocus: false,
    queryFn: async () =>
      helpAssignmentSchema.parse(
        await apiClient(`/admin/platform-bots/${profile}/help-library`, {
          authorityGuard: guard(user?.id),
        }),
      ),
  });
}
export function useAssignHelp(profile: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: async (body: {
      generation: number;
      version: number;
      library_id: string;
    }) =>
      helpAssignmentSchema.parse(
        await apiClient(`/admin/platform-bots/${profile}/help-library`, {
          method: "PUT",
          body,
          authorityGuard: guard(actor),
        }),
      ),
    onSuccess: () =>
      client.invalidateQueries({
        queryKey: ["platform-help-assignment", actor, profile],
      }),
  });
}
