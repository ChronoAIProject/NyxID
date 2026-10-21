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
  const actor = useAuthStore((state) => state.user?.id);
  return <OwnedBindings key={actor ?? "signed-out"} />;
}
function OwnedBindings() {
  const query = usePersonalBindings();
  const revoke = useRevokePersonalBinding();
  const [review, setReview] = useState<PersonalBinding>();
  const rows = query.data?.pages.flatMap((page) => page.bindings) ?? [];
  return (
    <section aria-label="Personal chat links" className="space-y-3 text-12">
      <p>
        These platform-operated chat bots use your existing NyxBot permissions
        and personal billing. Each verified identity has its own thread.
      </p>
      {query.isPending && <p role="status">Loading personal links...</p>}
      {query.error && (
        <ErrorBanner
          message="Personal chat links are unavailable."
          onRetry={() => void query.refetch()}
        />
      )}
      {query.isSuccess && rows.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-4 text-muted-foreground">
          No personal chat links. Use the login control shown by a platform
          bot's private chat help to begin.
        </p>
      )}
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
          <div
            key={row.id}
            className="space-y-2 rounded-lg border border-border p-4"
          >
            <p className="font-medium">{row.manager.display_name}</p>
            <p className="text-muted-foreground">
              {row.chat.display_name} account{" "}
              {row.sender.display_name
                ? `${row.sender.display_name} (${row.sender.id})`
                : row.sender.id}{" "}
              · {label}
            </p>
            {availability === "temporarily_unavailable" && (
              <p>
                The bot connection is unavailable. Your consent is retained and
                access can return when the connection is restored. Disconnect
                personal access to prevent that restoration.
              </p>
            )}
            {availability === "paused" && (
              <p>
                Personal work is paused for this bot. Your consent is retained.
                Disconnect personal access to prevent it returning when the bot
                resumes.
              </p>
            )}
            {availability === "reconnect_required" && (
              <p>
                This saved link no longer grants personal access. Start a fresh
                login request in the bot's private chat to reconnect.
              </p>
            )}
            {row.conversation_id ? (
              <a
                className="underline"
                href={`/assistant?c=${encodeURIComponent(row.conversation_id)}`}
              >
                Open saved {row.chat.display_name} thread
              </a>
            ) : availability === "available" ? (
              <p>
                {row.chat.question_instruction} This creates its saved thread.
              </p>
            ) : null}
            {consentActive && (
              <div>
                <Button variant="outline" onClick={() => setReview(row)}>
                  Disconnect personal access
                </Button>
              </div>
            )}
          </div>
        );
      })}
      {query.hasNextPage && (
        <Button
          variant="outline"
          onClick={() => void query.fetchNextPage()}
          isLoading={query.isFetchingNextPage}
        >
          More personal links
        </Button>
      )}
      {revoke.error && <ErrorBanner message={revoke.error.message} />}
      <Dialog
        open={!!review}
        onOpenChange={(open) => {
          if (!open) setReview(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              Disconnect personal {review?.chat.display_name ?? "chat"} access?
            </DialogTitle>
            <DialogDescription>
              Work already started may keep running in NyxID; stop it from the
              app. Your saved thread and pending approvals remain available.
              This disconnects only personal access for this identity.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setReview(undefined)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              isLoading={revoke.isPending}
              onClick={() => {
                if (review)
                  revoke.mutate(review, {
                    onSuccess: () => setReview(undefined),
                  });
              }}
            >
              Disconnect
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
