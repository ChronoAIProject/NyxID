import { useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Link2, Pencil, X } from "lucide-react";
import { toast } from "sonner";

export function CallbackUrlCard({
  keyId,
  callbackUrl,
}: {
  readonly keyId: string;
  readonly callbackUrl: string | null;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(callbackUrl ?? "");
  const updateApiKey = useUpdateApiKey();

  function handleSave() {
    const trimmed = value.trim();
    updateApiKey.mutate(
      { keyId, callback_url: trimmed.length > 0 ? trimmed : null },
      {
        onSuccess: () => {
          toast.success("Callback URL updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to update callback URL";
          toast.error(message);
        },
      },
    );
  }

  function handleClear() {
    updateApiKey.mutate(
      { keyId, callback_url: null },
      {
        onSuccess: () => {
          toast.success("Callback URL removed");
          setValue("");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to remove callback URL";
          toast.error(message);
        },
      },
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Link2 className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Callback URL</CardTitle>
        </div>
        <CardDescription>
          Where NyxID sends channel relay messages for this agent
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {editing ? (
          <div className="space-y-2">
            <Input
              type="url"
              placeholder="https://my-agent.example.com/webhook"
              value={value}
              onChange={(e) => setValue(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              Must be HTTPS in production. Used by Channel Bot Relay to forward
              platform messages to this agent.
            </p>
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={handleSave}
                disabled={updateApiKey.isPending}
              >
                Save
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setValue(callbackUrl ?? "");
                  setEditing(false);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            {callbackUrl ? (
              <code className="truncate rounded bg-muted px-2 py-1 text-xs">
                {callbackUrl}
              </code>
            ) : (
              <span className="text-sm text-muted-foreground">Not set</span>
            )}
            <div className="flex shrink-0 gap-1">
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={() => {
                  setValue(callbackUrl ?? "");
                  setEditing(true);
                }}
              >
                <Pencil className="h-3 w-3" />
              </Button>
              {callbackUrl && (
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6 text-destructive"
                  onClick={handleClear}
                  disabled={updateApiKey.isPending}
                >
                  <X className="h-3 w-3" />
                </Button>
              )}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
