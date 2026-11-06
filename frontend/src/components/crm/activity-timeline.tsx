import { CrmPeopleLabels, CrmPersonName } from "@/components/crm/people-picker";
import { useRef } from "react";
import {
  FilePlus2,
  MessageSquareText,
  MoveRight,
  PencilLine,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Form, useAppForm } from "@/components/ui/form";
import { Textarea } from "@/components/ui/textarea";
import { useAddCrmNote, useCrmActivityHistory } from "@/hooks/use-crm";
import type { CrmActivity } from "@/schemas/crm";

function ActivityIcon({ kind }: { readonly kind: CrmActivity["kind"] }) {
  if (kind === "created") return <FilePlus2 />;
  if (kind === "stage_changed") return <MoveRight />;
  if (kind === "note") return <MessageSquareText />;
  return <PencilLine />;
}

function activityText(activity: CrmActivity) {
  if (activity.kind === "created") return "Record created";
  if (activity.kind === "stage_changed") {
    return `Stage changed from ${activity.summary.from ?? "None"} to ${activity.summary.to ?? "None"}`;
  }
  if (activity.kind === "updated") {
    const count = activity.summary.changed_fields?.length ?? 0;
    return count
      ? `Updated ${count} ${count === 1 ? "field" : "fields"}`
      : "Record updated";
  }
  return activity.body ?? "Note added";
}

export function CrmActivityTimeline({
  recordId,
  ownerId,
  canWrite = true,
}: {
  readonly recordId: string;
  readonly ownerId?: string;
  readonly canWrite?: boolean;
}) {
  const activities = useCrmActivityHistory(recordId, ownerId);
  const addNote = useAddCrmNote(ownerId);
  const actionKey = useRef<string | null>(null);
  const form = useAppForm<{ body: string }>({ defaultValues: { body: "" } });
  const bodyField = form.register("body", { required: true });
  const history = [
    ...new Map(
      activities.data?.pages.flatMap((page) =>
        page.activities.map((activity) => [activity.id, activity] as const),
      ),
    ).values(),
  ];
  return (
    <CrmPeopleLabels
      ownerId={ownerId}
      ids={history.map((activity) => activity.actor.user_id)}
    >
      <section className="space-y-3" aria-labelledby="crm-activity-title">
        <h3 id="crm-activity-title" className="type-secondary-strong">
          Activity
        </h3>
        {canWrite ? (
          <Form {...form}>
            <form
              className="space-y-2"
              onSubmit={form.handleSubmit(async ({ body }) => {
                try {
                  actionKey.current ??= crypto.randomUUID();
                  await addNote.mutateAsync({
                    id: recordId,
                    body,
                    idempotencyKey: actionKey.current,
                  });
                  actionKey.current = null;
                  form.reset({ body: "" });
                  await activities.refetch();
                } catch {
                  // The mutation exposes the failure below; retain the draft and retry key.
                }
              })}
            >
              <Textarea
                disabled={addNote.isPending}
                aria-label="Add a note"
                placeholder="Add a note..."
                maxLength={10_000}
                {...bodyField}
                onChange={(event) => {
                  actionKey.current = null;
                  void bodyField.onChange(event);
                }}
              />
              <div className="flex justify-end">
                <Button
                  type="submit"
                  size="sm"
                  variant="primary"
                  disabled={
                    !form.formState.isDirty || !form.watch("body").trim()
                  }
                  isLoading={addNote.isPending}
                >
                  Add note
                </Button>
              </div>
              {addNote.isError ? (
                <p role="alert" className="type-meta text-destructive">
                  Could not add the note.
                </p>
              ) : null}
            </form>
          </Form>
        ) : null}
        {activities.isPending ? (
          <p role="status" className="type-meta text-text-tertiary">
            Loading activity...
          </p>
        ) : null}
        {activities.isError ? (
          <div role="alert" className="space-y-2 type-meta text-destructive">
            <p>Could not load activity.</p>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void activities.refetch()}
            >
              Retry history
            </Button>
          </div>
        ) : null}
        <ol className="space-y-3">
          {history.map((activity) => (
            <li
              key={activity.id}
              className="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-2"
            >
              <span
                className="mt-0.5 flex size-6 items-center justify-center rounded-md border border-border/50 bg-overlay text-text-tertiary [&_svg]:size-3"
                aria-hidden="true"
              >
                <ActivityIcon kind={activity.kind} />
              </span>
              <div className="min-w-0">
                <p className="whitespace-pre-wrap text-12 text-foreground">
                  {activityText(activity)}
                </p>
                {activity.kind === "stage_changed" &&
                (activity.summary.changed_fields?.length ?? 0) > 1 ? (
                  <p className="type-meta text-text-tertiary">
                    Changed fields:{" "}
                    {activity.summary.changed_fields?.join(", ")}
                  </p>
                ) : null}
                <p className="type-meta text-text-tertiary">
                  <CrmPersonName id={activity.actor.user_id} /> ·{" "}
                  {activity.actor.kind === "agent"
                    ? (activity.actor.agent_name ??
                      `Specialist ${activity.actor.agent_id ?? "unavailable"}`)
                    : "Human"}{" "}
                  · {new Date(activity.created_at).toLocaleString()}
                  {activity.actor.source ? (
                    <span className="block">
                      {activity.actor.source.run_id
                        ? `Scheduled run ${activity.actor.source.run_id}`
                        : "Assistant turn"}{" "}
                      · source time{" "}
                      {new Date(
                        activity.actor.source.ordered_at,
                      ).toLocaleString()}
                    </span>
                  ) : null}
                </p>
              </div>
            </li>
          ))}
        </ol>
        {activities.hasNextPage ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            isLoading={activities.isFetchingNextPage}
            onClick={() => void activities.fetchNextPage()}
          >
            {activities.isFetchNextPageError
              ? "Retry older activity"
              : "Load older activity"}
          </Button>
        ) : null}
      </section>
    </CrmPeopleLabels>
  );
}
