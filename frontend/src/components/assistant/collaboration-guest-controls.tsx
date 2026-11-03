import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { collaborationApi } from "@/hooks/use-collaboration-groups";
import { guestAccessRequestSchema } from "@/schemas/collaboration-group";
import { Badge } from "@/components/ui/badge";
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
      request: guestAccessRequestSchema.nullable().optional(),
    }),
  ),
});
export function CollaborationGuestControls({
  agentId,
  disabled = false,
}: {
  readonly agentId: string;
  readonly disabled?: boolean;
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
        ...(!row.enabled && row.request?.status === "pending"
          ? { request_id: row.request.id }
          : {}),
      }),
    onSuccess: () => {
      setConfirm(undefined);
      return client.invalidateQueries({ queryKey: key });
    },
  });
  const cancel = useMutation({
    mutationFn: (row: z.infer<typeof schema>["groups"][number]) =>
      collaborationApi.cancelGuestRequest(
        row.group_id,
        agentId,
        row.request!.id,
      ),
    onSettled: () => client.invalidateQueries({ queryKey: key }),
  });
  if (!query.data?.length) return null;
  return (
    <section className="space-y-2 text-12" aria-label="Group guest replies">
      <GroupedList
        title="Group guest replies"
        footer="Allow guests separately for each group. This gives you no access to its transcript or participants."
      >
        {query.data.map((row) => (
          <GroupedRow key={row.group_id} className="block space-y-2">
            <p className="min-w-0 break-all">Group {row.group_id}</p>
            {row.request ? (
              <Badge variant="secondary">Request {row.request.status}</Badge>
            ) : null}
            {row.request?.status === "pending" && !row.enabled ? (
              <>
                <p className="text-muted-foreground">
                  Allow current and future participants without use rights to
                  use this agent under your guest service policy. They use your
                  identity and billing; they cannot approve cards or use private
                  account tools.
                </p>
                <p className="text-muted-foreground">
                  Expires {new Date(row.request.expires_at).toLocaleString()}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={disabled || cancel.isPending}
                    isLoading={mutation.isPending}
                    onClick={() => mutation.mutate(row)}
                  >
                    Approve participant access
                  </Button>
                  <Button
                    size="sm"
                    disabled={disabled || mutation.isPending}
                    isLoading={cancel.isPending}
                    onClick={() => cancel.mutate(row)}
                  >
                    Cancel request
                  </Button>
                </div>
              </>
            ) : (
              <Button
                size="sm"
                isLoading={mutation.isPending}
                disabled={disabled}
                onClick={() =>
                  row.enabled ? mutation.mutate(row) : setConfirm(row)
                }
              >
                {row.enabled ? "Disable guests" : "Enable guests"}
              </Button>
            )}
          </GroupedRow>
        ))}
      </GroupedList>
      {mutation.error || cancel.error ? (
        <p role="alert" className="text-destructive">
          {(mutation.error ?? cancel.error)?.message}
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
