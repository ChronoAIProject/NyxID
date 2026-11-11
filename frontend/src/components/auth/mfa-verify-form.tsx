import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import type { Control } from "react-hook-form";
import { useRef, useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate } from "@tanstack/react-router";
import { mfaLoginSchema, type MfaVerifyFormData } from "@/schemas/auth";
import { useMfaVerify } from "@/hooks/use-auth";
import { useAuthStore } from "@/stores/auth-store";
import { apiClient, ApiError } from "@/lib/api-client";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { ShieldCheck } from "lucide-react";

interface MfaVerifyFormProps {
  readonly returnTo?: string;
  readonly cookieChallenge?: boolean;
}

export function MfaVerifyForm({ returnTo, cookieChallenge = false }: MfaVerifyFormProps) {
  useUiTranslation();
  const navigate = useNavigate();
  const verifyMutation = useMfaVerify();
  const mfaToken = useAuthStore((s) => s.mfaToken);
  const clearMfaState = useAuthStore((s) => s.clearMfaState);
  const codeInputRef = useRef<HTMLInputElement>(null);
  const [recoveryMode, setRecoveryMode] = useState(false);

  const form = useAppForm<MfaVerifyFormData>({
    resolver: zodResolver(mfaLoginSchema),
    defaultValues: {
      code: "",
    },
  });

  useEffect(() => {
    codeInputRef.current?.focus();
  }, []);

  async function onSubmit(data: MfaVerifyFormData) {
    if (!cookieChallenge && !mfaToken) return;
    try {
      await verifyMutation.mutateAsync({
        code: data.code,
        ...(!cookieChallenge && mfaToken ? { mfa_token: mfaToken } : {}),
      });
      const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
      if (trustedReturnTo) {
        window.location.assign(trustedReturnTo);
        return;
      }
      void navigate({ to: "/assistant" as string });
    } catch (error) {
      if (error instanceof ApiError) {
        const details = error.errorResponse.details;
        if (details && typeof details === "object" && "reason" in details && details.reason === "mfa_challenge_expired") {
          clearMfaState();
          const params = new URLSearchParams({ mfa: "expired" });
          const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
          if (trustedReturnTo) params.set("return_to", trustedReturnTo);
          window.location.assign(`/login?${params}`);
          return;
        }
        form.setError("root", { message: error.message });
      } else {
        form.setError("root", {
          message: "Verification failed. Please try again.",
        });
      }
    }
  }

  async function handleCancel() {
    if (cookieChallenge) {
      try {
        await apiClient("/auth/mfa/cancel", { method: "POST" });
      } catch {
        form.setError("root", { message: "Could not cancel verification. Please try again." });
        return;
      }
    }
    clearMfaState();
    if (cookieChallenge) {
      const params = new URLSearchParams();
      const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
      if (trustedReturnTo) params.set("return_to", trustedReturnTo);
      window.location.assign(`/login${params.size ? `?${params}` : ""}`);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-center space-y-2 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
          <ShieldCheck className="h-6 w-6 text-primary-text" aria-hidden="true" />
        </div>
        <h1 className="type-metric tracking-tight">{translateUi("MFA verification")}</h1>
        <p className="text-14 unthemed:text-12 text-muted-foreground">
          {verifyMutation.sessionConfirmationPending
            ? translateUi("Verification was submitted. Confirm your session to continue.")
            : cookieChallenge
              ? translateUi("Your social sign-in needs one more verification step.")
              : translateUi("Enter your authenticator code or a single-use recovery code")}
        </p>
      </div>

      <Form {...form}>
        <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
          {form.formState.errors.root && (
            <div
              role="alert"
              className="rounded-lg bg-destructive/10 p-3 text-14 unthemed:text-12 text-destructive"
            >
              {form.formState.errors.root.message}
            </div>
          )}

          <MfaCodeFields
            control={form.control}
            recoveryMode={recoveryMode}
            onToggleRecovery={() => setRecoveryMode((value) => !value)}
            disabled={verifyMutation.sessionConfirmationPending}
            inputRef={codeInputRef}
          />

          <div className="flex flex-col gap-2">
            <Button
              variant="primary"
              type="submit"
              className="w-full"
              isLoading={verifyMutation.isPending}
            >
              {verifyMutation.sessionConfirmationPending ? translateUi("Retry session check") : translateUi("Verify")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="w-full"
              onClick={handleCancel}
            >{translateUi("Back to Login")}</Button>
          </div>
        </form>
      </Form>
    </div>
  );
}

/** Shared authenticator/recovery input for sign-in and current-session step-up. */
export function MfaCodeFields({
  control,
  recoveryMode,
  onToggleRecovery,
  disabled = false,
  inputRef,
}: {
  control: Control<MfaVerifyFormData>;
  recoveryMode: boolean;
  onToggleRecovery: () => void;
  disabled?: boolean;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  useUiTranslation();
  return (
    <>
      <FormField
        control={control}
        name="code"
        render={({ field }) => (
          <FormItem>
            <FormLabel>
              {recoveryMode ? translateUi("Recovery code") : translateUi("Authenticator code")}
            </FormLabel>
            <FormControl>
              <Input
                placeholder={recoveryMode ? translateUi("Paste a recovery code") : "000000"}
                maxLength={32}
                autoComplete="one-time-code"
                inputMode={recoveryMode ? "text" : "numeric"}
                disabled={disabled}
                aria-label={
                  recoveryMode ? translateUi("Recovery code") : translateUi("Authenticator code")
                }
                className="min-w-0 font-mono text-14 tracking-normal sm:text-14"
                {...field}
                ref={inputRef ?? field.ref}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />

      <Button
        type="button"
        variant="link"
        className="h-auto p-0 text-14 unthemed:text-12"
        disabled={disabled}
        onClick={onToggleRecovery}
      >
        {recoveryMode ? translateUi("Use an authenticator code") : translateUi("Use a recovery code")}
      </Button>
    </>
  );
}
