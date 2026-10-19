import { useAuthStore } from "@/stores/auth-store";
import { useState } from "react";
import {
  usePersonalBindings,
  useRevokePersonalBinding,
} from "@/hooks/use-platform-personal";
import { personalPlatformLabel } from "@/schemas/platform-personal";
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
    <section aria-label="Linked channels" className="space-y-3 text-12">
      <p>
        These platform-operated messaging bots use your existing NyxBot
        permissions and personal billing. Each verified identity has its own
        thread.
      </p>
      {query.isPending && <p role="status">Loading personal links...</p>}
      {query.error && (
        <ErrorBanner
          message="Linked channels are unavailable."
          onRetry={() => void query.refetch()}
        />
      )}
      {query.isSuccess && rows.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-4 text-muted-foreground">
          No linked channels. Request a personal link in a shared bot's private
          conversation to begin.
        </p>
      )}
      {rows.map((row) => (
        <div
          key={row.id}
          className="space-y-2 rounded-lg border border-border p-4"
        >
          <p className="font-medium">{row.manager.display_name}</p>
          <p className="break-all text-muted-foreground">
            {personalPlatformLabel(row.manager.platform)} account{" "}
            {row.sender.display_name
              ? `${row.sender.display_name} (${row.sender.id})`
              : row.sender.id} ·{" "}
            {row.state === "active" ? "Linked" : "Disconnected"}
          </p>
          <p className="break-all text-muted-foreground">
            Private conversation: {row.sender.conversation_id ?? row.sender.id}
          </p>
          {row.conversation_id ? (
            <a
              className="underline"
              href={`/assistant?c=${encodeURIComponent(row.conversation_id)}`}
            >
              Open saved personal thread
            </a>
          ) : row.state === "active" ? (
            <p>
              {row.manager.platform === "discord"
                ? "Use /chat message in the bot DM to create your personal thread."
                : "Send your first private text message to create your personal thread."}
            </p>
          ) : null}
          {row.state === "active" && (
            <div>
              <Button variant="outline" onClick={() => setReview(row)}>
                Disconnect personal access
              </Button>
            </div>
          )}
        </div>
      ))}
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
            <DialogTitle>Disconnect personal channel access?</DialogTitle>
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
