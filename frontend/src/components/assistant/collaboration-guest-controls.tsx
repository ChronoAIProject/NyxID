import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { collaborationApi } from "@/hooks/use-collaboration-groups";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
const schema = z.object({
  groups: z.array(
    z.object({
      group_id: z.string(),
      membership_revision: z.number(),
      guest_revision: z.number(),
      enabled: z.boolean(),
    }),
  ),
});
export function CollaborationGuestControls({
  agentId,
}: {
  readonly agentId: string;
}) {
  const flag = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const key = ["assistant", actor, "guest-groups", agentId];
  const query = useQuery({
    queryKey: key,
    enabled: flag && Boolean(actor),
    retry: false,
    queryFn: async () =>
      schema.parse(
        await assistantJson(
          `/assistant/nyxagent/agents/${encodeURIComponent(agentId)}/group-access`,
        ),
      ).groups,
  });
  const [confirm, setConfirm] =
    useState<z.infer<typeof schema>["groups"][number]>();
  const mutation = useMutation({
    mutationFn: (row: z.infer<typeof schema>["groups"][number]) =>
      collaborationApi.change(row.group_id, row.membership_revision, {
        action: "guest_access",
        agent_id: agentId,
        enabled: !row.enabled,
        guest_revision: row.guest_revision,
      }),
    onSuccess: () => {
      setConfirm(undefined);
      return client.invalidateQueries({ queryKey: key });
    },
  });
  if (!query.data?.length) return null;
  return (
    <section className="space-y-2 text-12" aria-label="Group guest replies">
      <p className="font-medium">Group guest replies</p>
      <p className="text-muted-foreground">
        Allow guests separately for each group. This gives you no access to its
        transcript or participants.
      </p>
      {query.data.map((row) => (
        <div
          className="flex items-center justify-between gap-2"
          key={row.group_id}
        >
          <span className="min-w-0 break-all">{row.group_id}</span>
          <Button
            size="sm"
            isLoading={mutation.isPending}
            onClick={() =>
              row.enabled ? mutation.mutate(row) : setConfirm(row)
            }
          >
            {row.enabled ? "Disable guests" : "Enable guests"}
          </Button>
        </div>
      ))}
      {mutation.error ? (
        <p role="alert" className="text-destructive">
          {mutation.error.message}
        </p>
      ) : null}
      {confirm ? (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setConfirm(undefined);
          }}
        >
          <DialogContent className="md:max-w-md">
            <DialogHeader>
              <DialogTitle>Enable guest replies?</DialogTitle>
              <DialogDescription>
                Current and future participants without use rights can use this
                agent under your guest service policy, and you pay. Applies only
                to group {confirm.group_id}.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button onClick={() => setConfirm(undefined)}>Cancel</Button>
              <Button
                variant="primary"
                isLoading={mutation.isPending}
                onClick={() => mutation.mutate(confirm)}
              >
                Enable guests
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      ) : null}
    </section>
  );
}
