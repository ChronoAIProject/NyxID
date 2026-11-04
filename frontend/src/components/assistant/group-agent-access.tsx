import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { GroupedFooter, GroupedRow } from "@/components/ui/grouped-list";
import { useCollaborationGuestRequest } from "@/hooks/use-collaboration-groups";
import type {
  AssistantGroup,
  AssistantGroupMember,
} from "@/schemas/assistant-nyxagent";

export function GroupAgentAccessRequest({
  group,
  member,
  grouped = false,
}: {
  readonly group: AssistantGroup;
  readonly member: AssistantGroupMember;
  readonly grouped?: boolean;
}) {
  const mutation = useCollaborationGuestRequest(group.id);
  const policy = group.collaboration?.guest_access.find(
    (row) => row.agent_id === member.id,
  );
  const request = policy?.request;
  if (!policy) return null;
  if (grouped)
    return (
      <>
        {request?.status === "pending" ? (
          <GroupedRow
            title="Owner approval pending"
            description={`Expires ${new Date(request.expires_at).toLocaleDateString()}`}
            accessory={
              request.can_cancel ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={mutation.isPending}
                  onClick={() =>
                    mutation.mutate({ agent: member.id, cancelRequest: request.id })
                  }
                >
                  Cancel request
                </Button>
              ) : undefined
            }
          />
        ) : policy.can_request ? (
          <>
            {(request?.status === "expired" ||
              request?.status === "cancelled") && (
              <GroupedFooter>
                <p>Previous request {request.status}.</p>
              </GroupedFooter>
            )}
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={mutation.isPending}
                onClick={() => mutation.mutate({ agent: member.id })}
              >
                Ask {member.owner_name ?? "owner"} to allow participants
              </button>
            </GroupedRow>
          </>
        ) : null}
        {mutation.error && (
          <GroupedFooter>
            <p role="alert" className="text-destructive">
              {mutation.error.message}
            </p>
          </GroupedFooter>
        )}
      </>
    );
  return (
    <div className="space-y-1 text-14">
      {request?.status === "pending" ? (
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary">Owner approval pending</Badge>
          <span className="text-muted-foreground">
            Expires {new Date(request.expires_at).toLocaleDateString()}
          </span>
          {request.can_cancel ? (
            <Button
              size="sm"
              disabled={mutation.isPending}
              onClick={() =>
                mutation.mutate({ agent: member.id, cancelRequest: request.id })
              }
            >
              Cancel request
            </Button>
          ) : null}
        </div>
      ) : policy.can_request ? (
        <>
          {request?.status === "expired" || request?.status === "cancelled" ? (
            <p className="text-muted-foreground">
              Previous request {request.status}.
            </p>
          ) : null}
          <Button
            size="sm"
            isLoading={mutation.isPending}
            onClick={() => mutation.mutate({ agent: member.id })}
          >
            Ask {member.owner_name ?? "owner"} to allow participants
          </Button>
        </>
      ) : null}
      {mutation.error ? (
        <p role="alert" className="text-destructive">
          {mutation.error.message}
        </p>
      ) : null}
    </div>
  );
}
