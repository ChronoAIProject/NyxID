import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useState, useEffect, useCallback } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import QRCode from "qrcode";
import { mfaVerifySchema, type MfaVerifyFormData } from "@/schemas/auth";
import { useMfaSetup } from "@/hooks/use-auth";
import { api } from "@/lib/api-client";
import { ApiError } from "@/lib/api-client";
import type { MfaSetupResponse, MfaConfirmResponse } from "@/types/api";
import { useQueryClient } from "@tanstack/react-query";
import { copyToClipboard } from "@/lib/utils";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { Copy, Check, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

type MfaStep = "setup" | "verify" | "recovery";

interface MfaSetupDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

export function MfaSetupDialog({ open, onOpenChange }: MfaSetupDialogProps) {
  useUiTranslation();
  const [step, setStep] = useState<MfaStep>("setup");
  const [setupData, setSetupData] = useState<MfaSetupResponse | null>(null);
  const [recoveryCodes, setRecoveryCodes] = useState<readonly string[]>([]);
  const [qrDataUrl, setQrDataUrl] = useState<string>("");
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const setupMutation = useMfaSetup();
  const [setupPending, setSetupPending] = useState(false);
  const [setupError, setSetupError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  const form = useAppForm<MfaVerifyFormData>({
    resolver: zodResolver(mfaVerifySchema),
    defaultValues: {
      code: "",
    },
  });

  useEffect(() => {
    if (copiedIndex === null) return;
    const timer = setTimeout(() => setCopiedIndex(null), 2000);
    return () => clearTimeout(timer);
  }, [copiedIndex]);

  async function handleSetup() {
    setSetupPending(true);
    setSetupError(null);
    try {
      const data = await setupMutation.mutateAsync();
      setSetupData(data);
      const dataUrl = await QRCode.toDataURL(data.qr_code_url, {
        width: 200,
        margin: 4,
        color: {
          dark: "#0d0b14",
          light: "#f0eeff",
        },
      });
      setQrDataUrl(dataUrl);
      setStep("verify");
    } catch (error) {
      const message =
        error instanceof ApiError ? error.message : "Failed to set up MFA";
      setSetupError(message);
      toast.error(translateErrorMessage(message));
    } finally {
      setSetupPending(false);
    }
  }

  async function onVerify(data: MfaVerifyFormData) {
    form.clearErrors("root");
    try {
      const result = await api.post<MfaConfirmResponse>("/auth/mfa/confirm", {
        code: data.code,
      });
      setRecoveryCodes(result.recovery_codes);
      void queryClient.invalidateQueries({ queryKey: ["user"] });
      setStep("recovery");
      toast.success(translateUi("MFA enabled successfully"));
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

  const handleCopyCode = useCallback(async (code: string, index: number) => {
    try {
      await copyToClipboard(code);
      setCopiedIndex(index);
    } catch {
      toast.error(translateErrorMessage("Failed to copy to clipboard"));
    }
  }, []);

  const pending = setupPending || form.formState.isSubmitting;

  function handleClose() {
    if (pending) return;
    setSetupError(null);
    setStep("setup");
    setSetupData(null);
    setRecoveryCodes([]);
    setQrDataUrl("");
    form.reset();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="md:max-w-[520px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck
              className="h-5 w-5 text-primary-text"
              aria-hidden="true"
            />
            {step === "setup" && "Enable Two-Factor Authentication"}
            {step === "verify" && "Scan QR Code"}
            {step === "recovery" && "Recovery Codes"}
          </DialogTitle>
          <DialogDescription>
            {step === "setup" &&
              "Add an extra layer of security to your account."}
            {step === "verify" &&
              "Scan this QR code with your authenticator app, then enter the verification code."}
            {step === "recovery" &&
              "Save these recovery codes in a safe place. Each code works once if you lose your authenticator. Your other devices have been signed out."}
          </DialogDescription>
        </DialogHeader>

        {step === "setup" && (
          <div className="space-y-4" aria-busy={setupPending}>
            <p className="text-14 text-muted-foreground">{translateUi("You will need an authenticator app like Google Authenticator, Authy, or 1Password to complete setup.")}</p>
            {setupError && (
              <p role="alert" className="text-14 text-destructive">
                {setupError}
              </p>
            )}
            <Button
              variant="primary"
              onClick={handleSetup}
              className="w-full"
              isLoading={setupPending}
            >{translateUi("Begin Setup")}</Button>
          </div>
        )}

        {step === "verify" && (
          <div className="space-y-4">
            {qrDataUrl && (
              <div className="flex justify-center">
                <img
                  src={qrDataUrl}
                  alt={translateUi("Scan this QR code with your authenticator app to set up two-factor authentication")}
                  className="rounded-lg"
                  width={200}
                  height={200}
                />
              </div>
            )}

            {setupData?.secret && (
              <div className="space-y-1">
                <p className="text-13 text-muted-foreground">{translateUi("Or enter this code manually:")}</p>
                <code className="block rounded-lg bg-muted p-2 text-center font-mono text-14 select-all">
                  {setupData.secret}
                </code>
              </div>
            )}

            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onVerify)}
                aria-busy={form.formState.isSubmitting}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div
                    role="alert"
                    className="rounded-lg bg-destructive/10 p-3 text-14 text-destructive"
                  >
                    {form.formState.errors.root.message}
                  </div>
                )}

                <FormField
                  control={form.control}
                  name="code"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Verification Code")}</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="000000"
                          maxLength={6}
                          inputMode="numeric"
                          autoComplete="one-time-code"
                          aria-label={translateUi("Enter 6-digit verification code")}
                          className="text-center font-mono text-14 tracking-widest"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <Button
                  variant="primary"
                  type="submit"
                  className="w-full"
                  isLoading={form.formState.isSubmitting}
                  disabled={
                    !mfaVerifySchema.safeParse({ code: form.watch("code") })
                      .success || form.formState.isSubmitting
                  }
                >{translateUi("Verify and Enable")}</Button>
              </form>
            </Form>
          </div>
        )}

        {pending && (
          <p role="status" className="text-13 text-text-tertiary">
            {setupPending
              ? translateUi("Preparing two-factor authentication…")
              : translateUi("Verifying authentication code…")}
          </p>
        )}
        {step !== "recovery" && (
          <Button variant="outline" onClick={handleClose} disabled={pending}>{translateUi("Cancel")}</Button>
        )}

        {step === "recovery" && recoveryCodes.length > 0 && (
          <div className="space-y-4">
            <div
              className="grid min-w-0 grid-cols-1 gap-2"
              role="list"
              aria-label={translateUi("Recovery codes")}
            >
              {recoveryCodes.map((code, index) => (
                <button
                  key={code}
                  type="button"
                  onClick={() => void handleCopyCode(code, index)}
                  className="flex min-w-0 items-center justify-between gap-3 rounded-lg bg-muted px-3 py-2 text-left font-mono text-14 transition-colors duration-300 hover:bg-muted/80"
                  aria-label={translateUi("Copy recovery code {{value1}}", { value1: String(index + 1) })}
                >
                  <span className="min-w-0 break-all">{code}</span>
                  {copiedIndex === index ? (
                    <Check
                      className="h-3 w-3 shrink-0 text-success"
                      aria-hidden="true"
                    />
                  ) : (
                    <Copy
                      className="h-3 w-3 shrink-0 text-muted-foreground"
                      aria-hidden="true"
                    />
                  )}
                </button>
              ))}
            </div>

            <Button variant="primary" onClick={handleClose} className="w-full">{translateUi("Done")}</Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
