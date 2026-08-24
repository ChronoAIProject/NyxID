import { useRef, useState } from "react";
import { AppWindow, ShieldAlert } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import {
  assertSecretFreeReadBack,
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
  SECRET_VALUE_PATTERN,
} from "./assistant-action-dialog-utils";

// Exported as the single source of truth for the action union below.
// It is a real runtime value, not just a type: callers can validate an
// incoming action against it rather than trusting the compile-time type,
// which matters because the action is interpolated into the effect path.
export const developerActions = [
  "create",
  "update",
  "delete",
  "rotate_secret",
] as const;
export type AssistantDeveloperAppAction = (typeof developerActions)[number];

const responseSchema = z
  .object({
    resource: z.object({ clientId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    clientSecret: z.string().min(1).optional(),
  })
  .strict();

const evidenceSchema = z
  .object({
    id: z.string().min(1),
    broker_capability_enabled: z.boolean(),
    connection_webhook_enabled: z.boolean(),
    is_active: z.boolean(),
    created_at: z.string(),
    updated_at: z.string(),
  })
  .strict();

function textParam(params: Record<string, unknown>, key: string): string {
  const value = params[key];
  return typeof value === "string" ? value : "";
}

export function AssistantDeveloperAppActionDialog({
  open,
  onOpenChange,
  actionRequestId,
  action,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly action: AssistantDeveloperAppAction;
  readonly params: Record<string, unknown>;
  readonly onComplete: (clientId: string) => void;
}) {
  const [name, setName] = useState(textParam(params, "name"));
  const [redirectUris, setRedirectUris] = useState(
    Array.isArray(params.redirectUris) ? params.redirectUris.join("\n") : "",
  );
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ id: string; secret?: string } | null>(
    null,
  );
  const pendingRef = useRef(false);
  const destructive = action === "delete";
  const clientId = textParam(params, "clientId");

  function close() {
    pendingRef.current = false;
    setPending(false);
    setError(null);
    setConfirmed(false);
    setResult(null);
    onOpenChange(false);
  }

  async function readEvidence(id: string) {
    const raw = await api.get<unknown>(
      `/developer/oauth-clients/${encodeURIComponent(id)}/authorization`,
    );
    assertSecretFreeReadBack(raw);
    return evidenceSchema.parse(raw);
  }

  async function submit() {
    if (pendingRef.current || result) return;
    if (destructive && !confirmed) {
      setError("Confirm this destructive change to continue.");
      return;
    }
    if (
      (action === "create" || action === "update") &&
      SECRET_VALUE_PATTERN.test(`${name}\n${redirectUris}`)
    ) {
      setError("Application metadata cannot contain secret-shaped values.");
      return;
    }
    if (action === "create" && (!name.trim() || !redirectUris.trim())) {
      setError("Enter an application name and at least one redirect URI.");
      return;
    }
    if (action !== "create" && !clientId) {
      setError("The developer-app reference is missing.");
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const before = action === "create" ? null : await readEvidence(clientId);
      const reviewedRedirectUris = redirectUris
        .split(/\r?\n/)
        .map((uri) => uri.trim())
        .filter(Boolean);
      const payload: Record<string, unknown> = (() => {
        switch (action) {
          case "create":
            return {
              actionRequestId,
              name: name.trim(),
              redirectUris: reviewedRedirectUris,
            };
          case "update":
            return {
              actionRequestId,
              clientId,
              name: name.trim() || undefined,
              redirectUris: reviewedRedirectUris,
            };
          case "delete":
            return { actionRequestId, clientId, confirmed };
          case "rotate_secret":
            return {
              actionRequestId,
              clientId,
              expectedUpdatedAt: before?.updated_at,
            };
        }
      })();
      const raw = await api.post<unknown>(
        `/assistant/actions/org/developer-app/${action.replaceAll("_", "-")}`,
        payload,
      );
      const response = responseSchema.parse(raw);
      const id = response.resource.clientId;
      const after = await readEvidence(id);
      if (after.id !== id)
        throw new Error("NyxID returned a different developer-app identity.");
      if (action === "create" && !after.is_active) {
        throw new Error(
          "NyxID did not show the created application as active.",
        );
      }
      if (
        (action === "create" || action === "rotate_secret") &&
        !response.replayed &&
        !response.clientSecret
      ) {
        throw new Error(
          "NyxID did not return the one-time secret to the browser.",
        );
      }
      if (action === "delete" && after.is_active) {
        throw new Error("NyxID still reports this developer app as active.");
      }
      if (
        before &&
        !response.replayed &&
        !isNewerTimestamp(before.updated_at, after.updated_at)
      ) {
        throw new Error("NyxID did not show a newer developer-app state.");
      }
      setResult({
        id,
        ...(response.clientSecret ? { secret: response.clientSecret } : {}),
      });
    } catch (caught) {
      setError(
        errorMessage(
          caught,
          "NyxID could not complete this developer-app action.",
        ),
      );
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  const title = result
    ? "Developer app ready"
    : action === "rotate_secret"
      ? "Rotate developer-app secret"
      : `Developer app ${action}`;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AppWindow className="size-4" />
            {title}
          </DialogTitle>
          <DialogDescription>
            {result?.secret
              ? "Save this secret now. It will not be shown again."
              : result
                ? "The canonical developer-app projection confirms the change."
                : destructive
                  ? "Deleting the app revokes OAuth access and must be confirmed every time."
                  : "NyxID keeps client secrets out of chat."}
          </DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {result.secret ? (
              <div className="space-y-2">
                <Label htmlFor="developer-secret">One-time client secret</Label>
                <Input
                  id="developer-secret"
                  readOnly
                  value={result.secret}
                  className="font-mono"
                />
              </div>
            ) : null}
            <p className="font-mono text-xs text-muted-foreground">
              {result.id}
            </p>
          </div>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            {action === "create" || action === "update" ? (
              <>
                <div className="space-y-2">
                  <Label htmlFor="developer-name">Application name</Label>
                  <Input
                    id="developer-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="developer-redirects">Redirect URIs</Label>
                  <textarea
                    id="developer-redirects"
                    value={redirectUris}
                    onChange={(event) => setRedirectUris(event.target.value)}
                    className="min-h-20 w-full border border-input bg-background px-3 py-2 text-sm"
                  />
                </div>
              </>
            ) : (
              <p className="font-mono text-xs text-muted-foreground">
                {clientId}
              </p>
            )}
            {destructive ? (
              <label className="flex items-start gap-2 text-xs">
                <Checkbox
                  checked={confirmed}
                  onCheckedChange={(value) => setConfirmed(value === true)}
                />
                <span className="flex items-center gap-1">
                  <ShieldAlert className="size-3" />I understand this revokes
                  OAuth access.
                </span>
              </label>
            ) : null}
          </div>
        )}
        {error ? (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          {result ? (
            <Button
              type="button"
              onClick={() => {
                onComplete(result.id);
                close();
              }}
            >
              Done
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant={destructive ? "destructive" : "primary"}
                isLoading={pending}
                disabled={pending || (destructive && !confirmed)}
                onClick={() => void submit()}
              >
                {destructive ? "Delete app" : "Continue"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
