import { useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { useFeature } from "./use-feature-flag";
import { useAgentOwnerMutation } from "./use-agent-owner-mutation";
import {
  cmaSandboxSchema,
  cmaSandboxListSchema,
  cmaSandboxMutationSchema,
} from "@/schemas/nyxbot-cma-sandbox";

const BASE = "/assistant/nyxagent/sandboxes";
export const cmaQueryKeys = {
  root: (owner: string | undefined) => ["assistant", "cma", owner] as const,
  sandboxes: (owner: string | undefined) =>
    ["assistant", "cma", owner, "sandboxes"] as const,
  sandbox: (owner: string | undefined, id?: string) =>
    ["assistant", "cma", owner, "sandbox", id] as const,
};

export function useCmaSandboxes() {
  const owner = useAuthStore((state) => state.user?.id);
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  return useQuery({
    queryKey: cmaQueryKeys.sandboxes(owner),
    queryFn: async ({ signal }) => {
      const result = cmaSandboxListSchema.parse(
        await assistantJson(BASE, { signal }),
      );
      const seen = new Set<string>();
      let cursor = result.next_cursor;
      while (cursor && seen.size < 20 && !seen.has(cursor)) {
        seen.add(cursor);
        const page = cmaSandboxListSchema.parse(
          await assistantJson(`${BASE}?cursor=${encodeURIComponent(cursor)}`, {
            signal,
          }),
        );
        result.sandboxes.push(...page.sandboxes);
        result.conflict ||= page.conflict;
        result.inventory_complete &&= page.inventory_complete;
        cursor = page.next_cursor;
      }
      result.sandboxes = [
        ...new Map(result.sandboxes.map((row) => [row.id, row])).values(),
      ];
      result.next_cursor = cursor;
      if (cursor) result.inventory_complete = false;
      return result;
    },
    enabled: Boolean(owner) && enabled,
    retry: false,
    refetchInterval: 10_000,
  });
}

export function useCmaSandbox(id: string | undefined) {
  const owner = useAuthStore((state) => state.user?.id);
  const enabled = useFeature(FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES);
  return useQuery({
    queryKey: cmaQueryKeys.sandbox(owner, id),
    queryFn: async ({ signal }) =>
      cmaSandboxSchema.parse(
        await assistantJson(`${BASE}/${encodeURIComponent(id!)}`, { signal }),
      ),
    enabled: Boolean(owner && id) && enabled,
    retry: false,
    refetchInterval: 10_000,
  });
}

function useCmaMutation<TBody, TResult>(
  run: (body: TBody, authorityGuard: () => void) => Promise<TResult>,
) {
  const client = useQueryClient();
  return useAgentOwnerMutation({
    mutationFn: (body: TBody, ownerGuard) => {
      const guard = () => {
        ownerGuard();
        if (
          !userHasFeature(
            useAuthStore.getState().user,
            FEATURE_FLAG.ASSISTANT_CMA_SANDBOXES,
          )
        )
          throw new Error("Sandboxes are not enabled for this account.");
      };
      guard();
      return run(body, guard);
    },
    onSettled: (owner) =>
      client.invalidateQueries({ queryKey: cmaQueryKeys.root(owner) }),
  });
}

export function useRenameCmaSandbox() {
  return useCmaMutation(
    async (
      {
        id,
        generation,
        label,
      }: { id: string; generation: number; label: string },
      authorityGuard,
    ) =>
      cmaSandboxSchema.parse(
        await assistantJson(`${BASE}/${encodeURIComponent(id)}`, {
          method: "PATCH",
          body: { label, generation },
          authorityGuard,
        }),
      ),
  );
}
interface LifecycleRequest {
  id: string;
  generation: number;
  idempotencyKey: string;
}
type SandboxCommandRequest = Omit<LifecycleRequest, "idempotencyKey"> &
  (
    | { command: "pause" | "revive"; idempotencyKey: string }
    | { command: "recover_creation"; idempotencyKey?: never }
  );
export function useCmaSandboxCommand() {
  return useCmaMutation(
    async (
      { id, generation, command, idempotencyKey }: SandboxCommandRequest,
      authorityGuard,
    ) =>
      cmaSandboxMutationSchema.parse(
        await assistantJson(`${BASE}/${encodeURIComponent(id)}/commands`, {
          method: "POST",
          body: { command, generation },
          headers:
            command === "recover_creation"
              ? undefined
              : { "Idempotency-Key": idempotencyKey! },
          authorityGuard,
        }),
      ),
  );
}
export function useRetireCmaSandbox() {
  return useCmaMutation(
    async (
      { id, generation, idempotencyKey }: LifecycleRequest,
      authorityGuard,
    ) =>
      cmaSandboxMutationSchema.parse(
        await assistantJson(`${BASE}/${encodeURIComponent(id)}`, {
          method: "DELETE",
          body: { generation },
          headers: { "Idempotency-Key": idempotencyKey },
          authorityGuard,
        }),
      ),
  );
}
