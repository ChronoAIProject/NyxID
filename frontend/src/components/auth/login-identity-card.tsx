import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Fingerprint, Monitor } from "lucide-react";
import { Link } from "@tanstack/react-router";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { loginSchema, type LoginFormData } from "@/schemas/auth";
import { approvalMfaSchema } from "@/schemas/login-approval";
import type { useLoginApproval } from "@/hooks/use-login-approval";
import { LoginMethods } from "./login-methods";

export function LoginIdentityCard({
  approval,
  onVerified,
  disabled,
}: {
  approval: ReturnType<typeof useLoginApproval>;
  onVerified: () => void;
  disabled: boolean;
}) {
  const [keep, setKeep] = useState(false);
  const [busy, setBusy] = useState(false);
  const [appOpen, setAppOpen] = useState(false);
  const [error, setError] = useState<string | null>(() =>
    window.location.hash === "#identity-error"
      ? "Identity verification was not completed. Please try again."
      : null,
  );
  const password = useAppForm<LoginFormData>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: "", password: "" },
  });
  const mfa = useAppForm<{ code: string }>({
    resolver: zodResolver(approvalMfaSchema),
    defaultValues: { code: "" },
  });
  async function run(work: () => Promise<void>) {
    if (busy || disabled) return;
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message
          : "Identity verification failed. Try again.",
      );
    } finally {
      setBusy(false);
    }
  }
  const blocked = busy || disabled || approval.restoring;
  return (
    <div className="space-y-4 border-t border-border pt-4">
      <div className="space-y-1">
        <h3 className="text-[13px] font-semibold">Verify your identity</h3>
        <p className="text-[12px] text-muted-foreground">
          Choose how to use this browser. You’ll choose the requesting device’s
          access next.
        </p>
      </div>
      <div
        role="group"
        aria-label="Browser sign-in"
        className="grid gap-2 sm:grid-cols-2"
      >
        {[
          {
            value: false,
            title: "Only for this request",
            text: "Finish without signing in to this browser.",
            Icon: Fingerprint,
          },
          {
            value: true,
            title: "Keep me signed in",
            text: "Also sign in to NyxID in this browser.",
            Icon: Monitor,
          },
        ].map(({ value, title, text, Icon }) => (
          <button
            key={title}
            type="button"
            aria-pressed={(approval.identity?.keep_signed_in ?? keep) === value}
            disabled={blocked || !!approval.identity || appOpen}
            onClick={() => setKeep(value)}
            className={`rounded-lg border p-3 text-left disabled:opacity-60 ${(approval.identity?.keep_signed_in ?? keep) === value ? "border-primary/60" : "border-border hover:bg-white/[0.03]"}`}
          >
            <span className="flex items-center gap-2 text-[12px] font-medium">
              <Icon className="size-4" />
              {title}
            </span>
            <span className="mt-1 block text-[11px] text-muted-foreground">
              {text}
            </span>
          </button>
        ))}
      </div>
      {error && (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      )}
      {approval.identity?.mfa_required ? (
        <Form {...mfa}>
          <form
            className="space-y-3"
            onSubmit={mfa.handleSubmit(
              ({ code }) =>
                void run(async () => {
                  const result = await approval.mfa(code);
                  mfa.reset();
                  if (result.verified) onVerified();
                }),
            )}
          >
            <FormField
              control={mfa.control}
              name="code"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Authenticator code</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      inputMode="numeric"
                      autoComplete="one-time-code"
                      maxLength={6}
                      disabled={blocked}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <Button
              type="submit"
              variant="primary"
              className="h-9 w-full rounded-md"
              isLoading={busy}
              disabled={blocked}
            >
              Verify & continue
            </Button>
          </form>
        </Form>
      ) : (
        <>
          <LoginMethods
            returnTo={`${window.location.pathname}${window.location.search}`}
            disabled={blocked}
            appTransport={approval.appTransport(keep)}
            onAppVerified={onVerified}
            onAppOpenChange={(open) => {
              setAppOpen(open);
              if (!open) void run(() => approval.reset());
            }}
            onSocial={(provider) =>
              void run(async () => {
                const result = await approval.beginSocial(keep);
                const params = new URLSearchParams({
                  approval_id: result.id,
                  return_to: window.location.href,
                });
                window.location.assign(
                  `/api/v1/auth/social/${provider}?${params}`,
                );
              })
            }
          >
            <Form {...password}>
              <form
                className="space-y-3"
                onSubmit={password.handleSubmit(
                  (values) =>
                    void run(async () => {
                      const result = await approval.authenticate(keep, values);
                      password.setValue("password", "");
                      if (result.verified) onVerified();
                    }),
                )}
              >
                <FormField
                  control={password.control}
                  name="email"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Email</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          type="email"
                          autoComplete="username"
                          disabled={blocked}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={password.control}
                  name="password"
                  render={({ field }) => (
                    <FormItem>
                      <div className="flex items-center justify-between">
                        <FormLabel>Password</FormLabel>
                        <Link
                          to={"/forgot-password" as string}
                          className="text-xs font-medium text-nyx-secondary-400 hover:text-nyx-300"
                        >
                          Forgot password?
                        </Link>
                      </div>
                      <FormControl>
                        <Input
                          {...field}
                          type="password"
                          autoComplete="current-password"
                          maxLength={128}
                          disabled={blocked}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button
                  type="submit"
                  variant="primary"
                  className="h-9 w-full rounded-md"
                  isLoading={busy}
                  disabled={blocked}
                >
                  {(approval.identity?.keep_signed_in ?? keep)
                    ? "Sign in & continue"
                    : "Verify & continue"}
                </Button>
              </form>
            </Form>
          </LoginMethods>
        </>
      )}
      {!!approval.identity && (
        <Button
          type="button"
          variant="link"
          disabled={blocked || appOpen}
          onClick={() =>
            void run(async () => {
              await approval.reset();
              setKeep(false);
              mfa.reset();
            })
          }
        >
          Change verification choice
        </Button>
      )}
    </div>
  );
}
