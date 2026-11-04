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
    <section className="space-y-2 text-14" aria-label="Group guest replies">
      {query.data.map((row) => (
        <GroupedList
          key={row.group_id}
          title={`Group ${row.group_id}`}
          info="Allow people in this group to use this agent under your guest service policy. This does not give you access to the group’s transcript or participants."
          footer={
            <>
              <p>
                Allow guests separately for each group. This gives you no access
                to its transcript or participants.
              </p>
              {row.request?.status === "pending" && !row.enabled ? (
                <>
                  <p>
                    Allow current and future participants without use rights to
                    use this agent under your guest service policy. They use
                    your identity and billing; they cannot approve cards or use
                    private account tools.
                  </p>
                  <p>
                    Expires {new Date(row.request.expires_at).toLocaleString()}
                  </p>
                </>
              ) : null}
              {mutation.error || cancel.error ? (
                <p role="alert" className="text-destructive">
                  {(mutation.error ?? cancel.error)?.message}
                </p>
              ) : null}
            </>
          }
        >
          <GroupedRow
            search={{ id: `group-guests-${row.group_id}`, title: "Guest replies", breadcrumb: `Access › Group ${row.group_id}`, keywords: row.enabled ? "Enabled" : "Disabled" }}
            title="Guest replies"
            accessory={row.enabled ? "Enabled" : "Disabled"}
          />
          {row.request ? (
            <GroupedRow
              title="Request"
              accessory={row.request.status.replaceAll("_", " ")}
            />
          ) : null}
          {row.request?.status === "pending" && !row.enabled ? (
            <>
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  disabled={disabled || cancel.isPending || mutation.isPending}
                  onClick={() => mutation.mutate(row)}
                >
                  Approve participant access
                </button>
              </GroupedRow>
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  disabled={disabled || mutation.isPending || cancel.isPending}
                  onClick={() => cancel.mutate(row)}
                >
                  Cancel request
                </button>
              </GroupedRow>
            </>
          ) : (
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={disabled || mutation.isPending}
                onClick={() =>
                  row.enabled ? mutation.mutate(row) : setConfirm(row)
                }
              >
                {row.enabled ? "Disable guests" : "Enable guests"}
              </button>
            </GroupedRow>
          )}
        </GroupedList>
      ))}

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
                variant="ghost"
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
