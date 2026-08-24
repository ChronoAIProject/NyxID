import { useRef, useState } from "react";
import { z } from "zod";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "@/lib/api-client";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";
import {
  assertSecretFreeReadBack,
  errorMessage,
  isNewerTimestamp,
  SECRET_VALUE_PATTERN,
} from "./assistant-action-dialog-utils";

const paramsSchema = z
  .object({
    displayName: z.string().max(200).optional(),
    avatarUrl: z.string().max(2048).optional(),
  })
  .strict()
  .refine((value) => value.displayName !== undefined || value.avatarUrl !== undefined);

const responseSchema = z
  .object({
    resource: z.object({ userId: actionControlIdentitySchema }).strict(),
    replayed: z.boolean(),
  })
  .strict();

const evidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    is_active: z.boolean(),
    mfa_enabled: z.boolean(),
    display_name_length: z.number().int().nonnegative().nullable().optional(),
    avatar_url_length: z.number().int().nonnegative().nullable().optional(),
    updated_at: z.string().datetime({ offset: true }),
  })
  .passthrough();

export type AssistantAccountProfileUpdateParams = z.infer<typeof paramsSchema>;

export function AssistantAccountProfileUpdateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantAccountProfileUpdateParams;
  readonly onComplete: (userId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verified, setVerified] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultUserId, setResultUserId] = useState<string | null>(null);

  function close() {
    submittingRef.current = false;
    setSubmitting(false);
    setVerified(false);
    setError(null);
    setResultUserId(null);
    onOpenChange(false);
  }

  async function submit() {
    if (submittingRef.current || resultUserId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = paramsSchema.parse(params);
      if (
        (expected.displayName && SECRET_VALUE_PATTERN.test(expected.displayName)) ||
        (expected.avatarUrl && SECRET_VALUE_PATTERN.test(expected.avatarUrl))
      ) {
        throw new Error("Profile values must not contain secret-shaped material.");
      }
      const beforeValue = await api.get<unknown>("/users/me/authorization");
      assertSecretFreeReadBack(beforeValue);
      const before = evidenceSchema.parse(beforeValue);
      const response = responseSchema.parse(
        await api.post<unknown>("/assistant/actions/org/account/profile-update", {
          actionRequestId,
          displayName: expected.displayName,
          avatarUrl: expected.avatarUrl,
        }),
      );
      const afterValue = await api.get<unknown>("/users/me/authorization");
      assertSecretFreeReadBack(afterValue);
      const after = evidenceSchema.parse(afterValue);
      if (
        after.id !== response.resource.userId ||
        !after.is_active ||
        !isNewerTimestamp(before.updated_at, after.updated_at) ||
        (expected.displayName !== undefined &&
          after.display_name_length !== expected.displayName.trim().length) ||
        (expected.avatarUrl !== undefined &&
          after.avatar_url_length !== expected.avatarUrl.trim().length)
      ) {
        throw new Error("NyxID could not verify the updated account profile.");
      }
      setResultUserId(response.resource.userId);
      setVerified(true);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not update the account profile."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{resultUserId ? "Profile updated" : "Update account profile"}</DialogTitle>
          <DialogDescription>
            {resultUserId
              ? "The account projection confirms this exact profile update."
              : "Review the display changes before they are saved."}
          </DialogDescription>
        </DialogHeader>
        {!resultUserId ? (
          <div className="space-y-2 border-y border-border py-4 text-[12px]">
            {params.displayName !== undefined ? (
              <div className="flex items-center justify-between gap-4">
                <span className="text-muted-foreground">Display name</span>
                <span className="max-w-[65%] truncate font-medium">{params.displayName}</span>
              </div>
            ) : null}
            {params.avatarUrl !== undefined ? (
              <div className="flex items-center justify-between gap-4">
                <span className="text-muted-foreground">Avatar URL</span>
                <Badge variant="secondary" className="max-w-[65%] truncate font-mono">
                  {params.avatarUrl}
                </Badge>
              </div>
            ) : null}
          </div>
        ) : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        {verified ? <p className="text-[11px] text-success">Account profile verified.</p> : null}
        <DialogFooter>
          {!resultUserId ? (
            <>
              <Button type="button" variant="outline" onClick={close}>Cancel</Button>
              <Button type="button" variant="primary" isLoading={submitting} disabled={submitting} onClick={() => void submit()}>
                Update profile
              </Button>
            </>
          ) : (
            <Button type="button" variant="primary" onClick={() => { onComplete(resultUserId); close(); }}>
              Done
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
