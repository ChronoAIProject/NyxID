import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { apiClient } from "@/lib/api-client";
import { mfaLoginSchema, type MfaVerifyFormData } from "@/schemas/auth";
import { useAppForm, Form } from "@/components/ui/form";
import { MfaCodeFields } from "./mfa-verify-form";
import { Button } from "@/components/ui/button";

export function MfaStepUpForm({
  message,
  onVerified,
  onCancel,
}: {
  message: string;
  onVerified: () => Promise<void>;
  onCancel: () => void;
}) {
  const [recovery, setRecovery] = useState(false);
  const form = useAppForm<MfaVerifyFormData>({
    resolver: zodResolver(mfaLoginSchema),
    defaultValues: { code: "" },
  });
  async function submit(data: MfaVerifyFormData) {
    try {
      await apiClient("/auth/mfa/step-up", {
        method: "POST",
        body: data,
        preserveSessionOn401: true,
      });
    } catch (error) {
      form.setError("root", {
        message:
          error instanceof Error
            ? error.message
            : "Verification failed. Try again.",
      });
      return;
    }
    form.reset();
    await onVerified();
  }
  return (
    <section
      className="rounded-xl border border-border bg-card p-4 text-left"
      aria-label="Verify this session"
    >
      <h2 className="text-15 font-semibold">Verify this session</h2>
      <p className="mt-2 text-12 text-muted-foreground">{message}</p>
      <Form {...form}>
        <form className="mt-4 space-y-3" onSubmit={form.handleSubmit(submit)}>
          {form.formState.errors.root && (
            <p role="alert" className="text-12 text-destructive">
              {form.formState.errors.root.message}
            </p>
          )}
          <MfaCodeFields
            control={form.control}
            recoveryMode={recovery}
            disabled={form.formState.isSubmitting}
            onToggleRecovery={() => {
              setRecovery(!recovery);
              form.setValue("code", "");
            }}
          />
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              disabled={form.formState.isSubmitting}
              onClick={onCancel}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              disabled={form.formState.isSubmitting}
              isLoading={form.formState.isSubmitting}
            >
              Verify & continue
            </Button>
          </div>
        </form>
      </Form>
    </section>
  );
}
