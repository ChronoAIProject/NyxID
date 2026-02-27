import { useRef, useEffect } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate } from "@tanstack/react-router";
import { mfaVerifySchema, type MfaVerifyFormData } from "@/schemas/auth";
import { useMfaVerify } from "@/hooks/use-auth";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { isTrustedRedirect } from "@/lib/urls";
import {
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
}

export function MfaVerifyForm({ returnTo }: MfaVerifyFormProps) {
  const navigate = useNavigate();
  const verifyMutation = useMfaVerify();
  const mfaToken = useAuthStore((s) => s.mfaToken);
  const clearMfaState = useAuthStore((s) => s.clearMfaState);
  const codeInputRef = useRef<HTMLInputElement>(null);

  const form = useForm<MfaVerifyFormData>({
    resolver: zodResolver(mfaVerifySchema),
    defaultValues: {
      code: "",
    },
  });

  useEffect(() => {
    codeInputRef.current?.focus();
  }, []);

  async function onSubmit(data: MfaVerifyFormData) {
    if (!mfaToken) return;
    try {
      await verifyMutation.mutateAsync({
        code: data.code,
        mfa_token: mfaToken,
      });
      if (returnTo && isTrustedRedirect(returnTo)) {
        window.location.assign(returnTo);
        return;
      }
      void navigate({ to: "/" as string });
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        form.setError("root", {
          message: "Verification failed. Please try again.",
        });
      }
    }
  }

  function handleCancel() {
    clearMfaState();
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-center space-y-2 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
          <ShieldCheck className="h-6 w-6 text-primary" aria-hidden="true" />
        </div>
        <h1 className="font-display text-[28px] font-normal tracking-tight">
          Two-factor authentication
        </h1>
        <p className="text-sm text-muted-foreground">
          Enter the 6-digit code from your authenticator app
        </p>
      </div>

      <Form {...form}>
        <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
          {form.formState.errors.root && (
            <div
              role="alert"
              className="rounded-md bg-destructive/10 p-3 text-sm text-destructive"
            >
              {form.formState.errors.root.message}
            </div>
          )}

          <FormField
            control={form.control}
            name="code"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Verification Code</FormLabel>
                <FormControl>
                  <Input
                    placeholder="000000"
                    maxLength={6}
                    autoComplete="one-time-code"
                    inputMode="numeric"
                    aria-label="Enter 6-digit verification code"
                    className="text-center font-mono text-lg tracking-widest"
                    {...field}
                    ref={codeInputRef}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />

          <div className="flex flex-col gap-2">
            <Button
              type="submit"
              className="w-full"
              isLoading={verifyMutation.isPending}
            >
              Verify
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="w-full"
              onClick={handleCancel}
            >
              Back to login
            </Button>
          </div>
        </form>
      </Form>
    </div>
  );
}
