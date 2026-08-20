import { useRef, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ApiError, api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  serviceDeleteActionParamsSchema,
} from "@/schemas/assistant-actions";

const GRANT_CASCADE_CODE = 11_500;

const serviceResourceSchema = z
  .object({ userServiceId: actionControlIdentitySchema })
  .strict();
const assistantServiceDeleteResponseSchema = z
  .object({
    resource: serviceResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

const cascadeDetailsSchema = z
  .object({
    provider_slug: z.string(),
    provider_name: z.string(),
    revokes_grant: z.boolean(),
    siblings: z.array(
      z
        .object({
          user_service_id: z.string(),
          name: z.string(),
          slug: z.string(),
        })
        .passthrough(),
    ),
    token_scope_available: z.boolean().optional(),
  })
  .passthrough();

function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

export interface AssistantServiceDeleteParams {
  readonly userServiceId: string;
}

export function AssistantServiceDeleteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantServiceDeleteParams;
  readonly onComplete: (userServiceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [cascade, setCascade] = useState<z.infer<
    typeof cascadeDetailsSchema
  > | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  function resetConfirmState() {
    setConfirmed(false);
    setCascade(null);
    setError(null);
    setDone(false);
    submittingRef.current = false;
    setSubmitting(false);
  }

  function close() {
    resetConfirmState();
    onOpenChange(false);
  }

  async function submit(cascadeGrant: boolean) {
    if (submittingRef.current || done) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = serviceDeleteActionParamsSchema.parse({
        userServiceId: params.userServiceId,
      });
      const response = assistantServiceDeleteResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/services/delete", {
          actionRequestId,
          userServiceId: expected.userServiceId,
          ...(cascadeGrant ? { cascadeGrant: true } : {}),
        }),
      );
      setDone(true);
      onComplete(response.resource.userServiceId);
    } catch (caught) {
      if (caught instanceof ApiError && caught.errorCode === GRANT_CASCADE_CODE) {
        const parsed = cascadeDetailsSchema.safeParse(caught.errorResponse.details);
        if (parsed.success) {
          setCascade(parsed.data);
          setError(null);
        } else {
          setError(
            errorMessage(
              caught,
              "This delete would revoke a shared OAuth grant. Confirm to continue.",
            ),
          );
        }
      } else {
        setError(errorMessage(caught, "NyxID could not delete this service."));
      }
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete connected service</DialogTitle>
          <DialogDescription>
            This cannot be undone from chat. Confirm every time — NyxID never
            skips the confirmation, including retries.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 border-y border-border py-4 text-[12px]">
          <p className="break-all font-mono text-muted-foreground">
            {params.userServiceId}
          </p>
          {cascade ? (
            <div className="space-y-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2">
              <p className="flex items-start gap-2 text-warning">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                Deleting this {cascade.provider_name} connection will also
                revoke the shared grant
                {cascade.siblings.length
                  ? ` used by ${String(cascade.siblings.length)} other service${cascade.siblings.length === 1 ? "" : "s"}`
                  : ""}
                .
              </p>
            </div>
          ) : (
            <p className="text-muted-foreground">
              The connected service, its stored credential, and its endpoint
              will be removed. Confirm explicitly to continue.
            </p>
          )}
        </div>

        {error ? (
          <p role="alert" className="text-[12px] text-destructive">
            {error}
          </p>
        ) : null}

        <DialogFooter>
          <Button type="button" variant="outline" onClick={close}>
            Cancel
          </Button>
          {!confirmed ? (
            <Button
              type="button"
              variant="destructive"
              onClick={() => setConfirmed(true)}
            >
              I understand, continue
            </Button>
          ) : (
            <Button
              type="button"
              variant="destructive"
              isLoading={submitting}
              onClick={() => void submit(cascade !== null)}
            >
              {cascade ? "Delete and revoke grant" : "Delete service"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
