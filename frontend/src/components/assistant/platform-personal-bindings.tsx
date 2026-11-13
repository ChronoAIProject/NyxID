import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import {
  GroupedList,
  GroupedRow,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import { useAuthStore } from "@/stores/auth-store";
import { useState } from "react";
import {
  usePersonalBindings,
  useRevokePersonalBinding,
} from "@/hooks/use-platform-personal";
import type { PersonalBinding } from "@/schemas/platform-personal";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

export function PlatformPersonalBindings() {
  useUiTranslation();
  const actor = useAuthStore((state) => state.user?.id);
  return <OwnedBindings key={actor ?? "signed-out"} />;
}
function OwnedBindings() {
  useUiTranslation();
  const query = usePersonalBindings();
  const revoke = useRevokePersonalBinding();
  const [review, setReview] = useState<PersonalBinding>();
  const rows = query.data?.pages.flatMap((page) => page.bindings) ?? [];
  return (
    <section className="space-y-3 text-14">
      <GroupedList
        title={translateUi("Personal chat links")}
        footer={
          <>
            <p>{translateUi("These platform-operated chat bots use your existing NyxBot permissions and personal billing. Each verified identity has its own thread.")}</p>
            {query.error && (
              <ErrorBanner
                message={translateUi("Personal chat links are unavailable.")}
                onRetry={() => void query.refetch()}
              />
            )}
            {revoke.error && <ErrorBanner message={revoke.error.message} />}
          </>
        }
      >
        {query.isPending && (
          <GroupedRow role="status" title={translateUi("Loading personal links...")} />
        )}
        {query.isSuccess && rows.length === 0 && (
          <GroupedRow
            title={translateUi("No personal chat links")}
            description={translateUi("Use the login control shown by a platform bot's private chat help to begin.")}
          />
        )}
        {query.hasNextPage && (
          <GroupedRow asChild interactive>
            <button
              type="button"
              onClick={() => void query.fetchNextPage()}
              disabled={query.isFetchingNextPage}
            >{translateUi("More personal links")}</button>
          </GroupedRow>
        )}
      </GroupedList>
      {rows.map((row) => {
        const consentActive = (row.consent_state ?? row.state) === "active";
        const availability = !consentActive
          ? "disconnected"
          : (row.availability ??
            (row.state === "active" ? "available" : "reconnect_required"));
        const label = {
          available: "Linked",
          temporarily_unavailable: "Temporarily unavailable",
          paused: "Personal access paused",
          reconnect_required: "Reconnect required",
          disconnected: "Disconnected",
        }[availability];
        return (
          <GroupedList key={row.id} title={row.manager.display_name}>
            <GroupedRow
              title={translateUi("{{value1}} account", { value1: row.chat.display_name })}
              description={
                row.sender.display_name
                  ? `${row.sender.display_name} (${row.sender.id})`
                  : row.sender.id
              }
              accessory={label}
            />
            <GroupedFooter>
              {availability === "temporarily_unavailable" && (
                <p>{translateUi("The bot connection is unavailable. Your consent is retained and access can return when the connection is restored. Disconnect personal access to prevent that restoration.")}</p>
              )}
              {availability === "paused" && (
                <p>{translateUi("Personal work is paused for this bot. Your consent is retained. Disconnect personal access to prevent it returning when the bot resumes.")}</p>
              )}
              {availability === "reconnect_required" && (
                <p>{translateUi("This saved link no longer grants personal access. Start a fresh login request in the bot's private chat to reconnect.")}</p>
              )}
            </GroupedFooter>
            {row.conversation_id ? (
              <GroupedRow asChild interactive>
                <a
                  href={`/assistant?c=${encodeURIComponent(row.conversation_id)}`}
                >{translateUi("Open saved ")}{row.chat.display_name}{translateUi(" thread")}</a>
              </GroupedRow>
            ) : availability === "available" ? (
              <GroupedFooter>
                <p>
                  {row.chat.question_instruction}{translateUi(" This creates its saved thread.")}</p>
              </GroupedFooter>
            ) : null}
            {consentActive && (
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  className="text-destructive"
                  onClick={() => setReview(row)}
                >{translateUi("Disconnect personal access")}</button>
              </GroupedRow>
            )}
          </GroupedList>
        );
      })}
      <Dialog
        open={!!review}
        onOpenChange={(open) => {
          if (!open) setReview(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Disconnect personal ")}{review?.chat.display_name ?? translateUi("chat")}{translateUi(" access?")}</DialogTitle>
            <DialogDescription>{translateUi("Work already started may keep running in NyxID; stop it from the app. Your saved thread and pending approvals remain available. This disconnects only personal access for this identity.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setReview(undefined)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={revoke.isPending}
              onClick={() => {
                if (review)
                  revoke.mutate(review, {
                    onSuccess: () => setReview(undefined),
                  });
              }}
            >{translateUi("Disconnect")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
