import { useState, useRef } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate, Link } from "@tanstack/react-router";
import {
  loginSchema,
  type LoginFormData,
  registerSchema,
  type RegisterFormData,
} from "@/schemas/auth";
import { useLogin, useRegister } from "@/hooks/use-auth";
import { ApiError } from "@/lib/api-client";
import { openExternal } from "@/lib/navigation";
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
import { toast } from "sonner";
import { usePublicConfig } from "@/hooks/use-public-config";
import { AUTH_PROVIDER_ICONS } from "@/components/auth/provider-icons";
import { LoginMethods } from "@/components/auth/login-methods";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function getPasswordStrength(password: string): {
  score: number;
  label: string;
  color: string;
} {
  let score = 0;
  if (password.length >= 8) score += 1;
  if (password.length >= 12) score += 1;
  if (/[A-Z]/.test(password)) score += 1;
  if (/[a-z]/.test(password)) score += 1;
  if (/[0-9]/.test(password)) score += 1;
  if (/[^A-Za-z0-9]/.test(password)) score += 1;

  if (score <= 2) return { score, label: "Weak", color: "bg-destructive" };
  if (score <= 4) return { score, label: "Fair", color: "bg-warning" };
  return { score, label: "Strong", color: "bg-success" };
}

/** Map backend social-auth error keys to user-friendly messages. */
const SOCIAL_ERROR_MESSAGES: Record<string, string> = {
  social_auth_conflict:
    "This social account is already linked elsewhere. Please use your original sign-in method or contact support.",
  social_auth_no_email:
    "We couldn't retrieve an email address from your social account. Please ensure your email is public or use email/password sign-in.",
  social_auth_deactivated:
    "Your account has been deactivated. Please contact support for assistance.",
  social_auth_failed: "Social sign-in failed. Please try again.",
  social_auth_exchange:
    "Social sign-in failed due to a temporary error. Please try again.",
};

// Social provider buttons for the register methods panel (full-width list style)
const REGISTER_PROVIDERS = [
  {
    id: "google",
    label: "Continue with Google",
    icon: AUTH_PROVIDER_ICONS.google,
  },
  {
    id: "github",
    label: "Continue with GitHub",
    icon: AUTH_PROVIDER_ICONS.github,
  },
  {
    id: "apple",
    label: "Continue with Apple",
    icon: AUTH_PROVIDER_ICONS.apple,
  },
] as const;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

type AuthPanel = 0 | 1 | 2;

interface AuthFlowProps {
  readonly initialPanel?: AuthPanel;
  readonly returnTo?: string;
  readonly socialError?: string;
}

export function AuthFlow({
  initialPanel = 0,
  returnTo,
  socialError,
}: AuthFlowProps) {
  const { data: publicConfig } = usePublicConfig();
  const emailAuthEnabled = publicConfig?.email_auth_enabled ?? false;

  const [panel, setPanel] = useState<AuthPanel>(
    initialPanel === 2 && !emailAuthEnabled ? 1 : initialPanel,
  );
  const [fadeOpacity, setFadeOpacity] = useState(1);
  const fadingRef = useRef(false);
  const navigate = useNavigate();
  const isLogin = panel === 0;
  const showEmailForm = panel === 2;
  // Refs for focus after slide
  const loginEmailRef = useRef<HTMLInputElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);

  // -- Forms --
  const loginForm = useAppForm<LoginFormData>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: "", password: "" },
  });

  const registerForm = useAppForm<RegisterFormData>({
    resolver: zodResolver(registerSchema),
    defaultValues: {
      name: "",
      email: "",
      password: "",
      confirmPassword: "",
    },
  });

  // -- Mutations --
  const loginMutation = useLogin();
  const registerMutation = useRegister();

  // -- Watched values --
  const regPassword = registerForm.watch("password");
  const strength = getPasswordStrength(regPassword);

  // Hide social providers whose backend credentials are not configured.
  // While publicConfig is loading, render none rather than flashing buttons
  // that may immediately disappear once the config arrives.
  const enabledProviders = publicConfig
    ? REGISTER_PROVIDERS.filter((p) => publicConfig.social_providers.includes(p.id))
    : [...REGISTER_PROVIDERS];

  // -- Slide helpers --
  const FADE_MS = 200;

  function slideToPanel(target: AuthPanel) {
    const currentIsLogin = panel === 0;
    const targetIsLogin = target === 0;
    const crossingLoginRegister = currentIsLogin !== targetIsLogin;

    if (crossingLoginRegister && !fadingRef.current) {
      // Sequential fade: out → swap → in
      fadingRef.current = true;
      setFadeOpacity(0);
      setTimeout(() => {
        setPanel(target);
        const path = target === 0 ? "/login" : "/register";
        const nextParams = new URLSearchParams();
        if (returnTo) nextParams.set("return_to", returnTo);
        const qs = nextParams.toString();
        window.history.replaceState(
          null,
          "",
          `${path}${qs ? `?${qs}` : ""}`,
        );
        // Small delay for React to render new content before fading in
        requestAnimationFrame(() => {
          setFadeOpacity(1);
          fadingRef.current = false;
        });
      }, FADE_MS);
    } else if (!crossingLoginRegister) {
      // Same view (register methods ↔ email form): instant panel switch
      setPanel(target);
    }

    setTimeout(() => {
      if (target === 0) loginEmailRef.current?.focus();
      else if (target === 2) nameInputRef.current?.focus();
    }, crossingLoginRegister ? FADE_MS * 2 + 50 : 350);
  }

  // -- Login submit --
  async function onLoginSubmit(data: LoginFormData) {
    try {
      const result = await loginMutation.mutateAsync(data);
      if (!result.mfaRequired) {
        const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
        if (trustedReturnTo) {
          window.location.assign(trustedReturnTo);
          return;
        }
        void navigate({ to: "/dashboard" as string });
      }
    } catch (error) {
      if (error instanceof ApiError) {
        loginForm.setError("root", { message: error.message });
      } else {
        loginForm.setError("root", {
          message: "An unexpected error occurred. Please try again.",
        });
      }
    }
  }

  // -- Register submit --
  async function onRegisterSubmit(data: RegisterFormData) {
    try {
      const result = await registerMutation.mutateAsync({
        display_name: data.name,
        email: data.email,
        password: data.password,
      });
      toast.info(result.message || "Check your email to complete registration.");
      slideToPanel(0);
    } catch (error) {
      if (error instanceof ApiError) {
        registerForm.setError("root", { message: error.message });
      } else {
        registerForm.setError("root", {
          message: "An unexpected error occurred. Please try again.",
        });
      }
    }
  }

  function handleRegisterSocialLogin(providerId: string) {
    const params = new URLSearchParams();
    if (returnTo) params.set("return_to", returnTo);
    const qs = params.toString();
    const url = `${window.location.origin}/api/v1/auth/social/${encodeURIComponent(providerId)}${qs ? `?${qs}` : ""}`;
    void openExternal(url);
  }

  return (
    <div
      className="overflow-hidden"
      style={{
        opacity: fadeOpacity,
        transition: `opacity ${FADE_MS}ms ease-in-out`,
      }}
    >
      {isLogin ? (
        /* ================================================================
           Login View
           ================================================================ */
        <div>
          <div className="mb-8">
            <h1 className="text-[28px] font-bold tracking-tight" style={{ letterSpacing: "-0.03em" }}>
              Welcome back
            </h1>
            <p className="mt-1 text-[13px] text-muted-foreground">
              Sign in to your account
            </p>
          </div>

          {socialError && (
            <div
              role="alert"
              data-testid="social-error"
              className="mb-4 rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
            >
              {SOCIAL_ERROR_MESSAGES[socialError] ??
                "Social sign-in failed. Please try again."}
            </div>
          )}

          <LoginMethods
            returnTo={returnTo}
            onSignUp={() => slideToPanel(1)}
            onSocial={(provider) => {
              const params = new URLSearchParams();
              if (returnTo) params.set("return_to", returnTo);
              const qs = params.toString();
              void openExternal(`${window.location.origin}/api/v1/auth/social/${provider}${qs ? `?${qs}` : ""}`);
            }}
          >
            <Form {...loginForm}>
              <form
                onSubmit={loginForm.handleSubmit(onLoginSubmit)}
                className="flex flex-col gap-4"
              >
                {loginForm.formState.errors.root && (
                  <div
                    role="alert"
                    className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
                  >
                    {loginForm.formState.errors.root.message}
                  </div>
                )}

                <FormField
                  control={loginForm.control}
                  name="email"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Email</FormLabel>
                      <FormControl>
                        <Input
                          type="email"
                          placeholder="you@example.com"
                          autoComplete="email"
                          {...field}
                          ref={(el) => {
                            field.ref(el);
                            (
                              loginEmailRef as React.MutableRefObject<HTMLInputElement | null>
                            ).current = el;
                          }}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={loginForm.control}
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
                          type="password"
                          placeholder="Enter your password"
                          autoComplete="current-password"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <Button
                  type="submit"
                  className="mt-1 h-[44px] w-full nyx-gradient-vivid text-[13px] font-medium shadow-[0_2px_12px_rgba(90,42,241,0.25)] hover:opacity-90 hover:shadow-[0_4px_20px_rgba(90,42,241,0.35)]"
                  isLoading={loginMutation.isPending}
                >
                  Sign in
                </Button>
              </form>
            </Form>
          </LoginMethods>
        </div>
      ) : (
        /* ================================================================
           Register View (2-panel slider: methods → email form)
           ================================================================ */
        <div className="overflow-hidden">
        <div
          className="flex w-[200%] items-start transition-transform duration-300 ease-in-out"
          style={{ transform: showEmailForm ? "translateX(-50%)" : "translateX(0)" }}
        >
        {/* Register Panel 1 — Method Selection */}
        <div className="w-1/2 shrink-0">
          <div className="mb-8">
            <h1 className="text-[28px] font-bold tracking-tight" style={{ letterSpacing: "-0.03em" }}>
              Create your account
            </h1>
            <p className="mt-1 text-[13px] text-muted-foreground">
              Start securing your digital identity
            </p>
          </div>

          <div>
            <p className="mb-3 text-[13px] font-medium leading-6 text-muted-foreground">
              Choose how to sign up
            </p>

            <div className="flex flex-col gap-2.5">
              {enabledProviders.map((provider) => (
                <button
                  key={provider.id}
                  type="button"
                  onClick={() => handleRegisterSocialLogin(provider.id)}
                  className="flex h-[44px] w-full cursor-pointer items-center justify-center gap-2.5 rounded-lg border border-border bg-transparent text-[13px] font-medium text-foreground transition-colors duration-200 hover:border-hairline-strong hover:bg-overlay active:scale-[0.99]"
                >
                  {provider.icon}
                  {provider.label}
                </button>
              ))}

              {emailAuthEnabled && (
                <button
                  type="button"
                  onClick={() => slideToPanel(2)}
                  className="flex h-[44px] w-full cursor-pointer items-center justify-center gap-2.5 rounded-lg border border-border bg-transparent text-[13px] font-medium text-foreground transition-colors duration-200 hover:border-hairline-strong hover:bg-overlay active:scale-[0.99]"
                >
                  <svg
                    className="h-4 w-4"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <rect x="2" y="4" width="20" height="16" rx="2" />
                    <path d="M22 7l-10 6L2 7" />
                  </svg>
                  Continue with Email
                </button>
              )}
            </div>
          </div>

          {/* Footer */}
          <div className="mt-8 text-center text-[13px] text-muted-foreground">
            Already have an account?{" "}
            <button
              type="button"
              onClick={() => slideToPanel(0)}
              className="cursor-pointer font-medium text-foreground underline underline-offset-2 hover:text-nyx-secondary-400"
            >
              Sign in
            </button>
          </div>
        </div>

        {/* Register Panel 2 — Email Registration */}
        <div
          className="w-1/2 shrink-0"
          onKeyDown={(e) => {
            if (e.key === "Escape") slideToPanel(1);
          }}
        >
          {/* Header with back button */}
          <div className="mb-6 flex items-center gap-3">
            <button
              type="button"
              onClick={() => slideToPanel(1)}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-transparent text-muted-foreground transition-colors duration-300 hover:border-border/80 hover:bg-overlay hover:text-foreground"
              aria-label="Back to sign-up methods"
            >
              <svg
                className="h-4 w-4"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M10 3L5 8l5 5" />
              </svg>
            </button>
            <div>
              <h2 className="text-lg font-semibold tracking-tight">
                Email registration
              </h2>
              <p className="text-xs text-muted-foreground">
                Create your account with email and password
              </p>
            </div>
          </div>

          {/* Email registration form */}
          <Form {...registerForm}>
            <form
              onSubmit={
                // eslint-disable-next-line react-hooks/refs -- handleSubmit only invokes onRegisterSubmit on submit events, never during render
                registerForm.handleSubmit(onRegisterSubmit)
              }
              className="flex flex-col gap-3.5"
            >
              {registerForm.formState.errors.root && (
                <div
                  role="alert"
                  className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
                >
                  {registerForm.formState.errors.root.message}
                </div>
              )}

              <FormField
                control={registerForm.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel className="text-xs">Full Name</FormLabel>
                    <FormControl>
                      <Input
                        placeholder="John Doe"
                        autoComplete="name"
                        className="h-[42px] text-[13.5px]"
                        {...field}
                        ref={(el) => {
                          field.ref(el);
                          (
                            nameInputRef as React.MutableRefObject<HTMLInputElement | null>
                          ).current = el;
                        }}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={registerForm.control}
                name="email"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel className="text-xs">Email</FormLabel>
                    <FormControl>
                      <Input
                        type="email"
                        placeholder="you@example.com"
                        autoComplete="email"
                        className="h-[42px] text-[13.5px]"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={registerForm.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel className="text-xs">Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        placeholder="Min 8 characters"
                        autoComplete="new-password"
                        className="h-[42px] text-[13.5px]"
                        {...field}
                      />
                    </FormControl>
                    {regPassword.length > 0 && (
                      <div className="mt-1.5 space-y-0.5">
                        <div
                          className="flex gap-[3px]"
                          role="progressbar"
                          aria-valuenow={strength.score}
                          aria-valuemin={0}
                          aria-valuemax={6}
                          aria-label={`Password strength: ${strength.label}`}
                        >
                          {Array.from({ length: 6 }).map((_, i) => (
                            <div
                              key={`s-${String(i)}`}
                              className={`h-[2.5px] flex-1 rounded-full ${
                                i < strength.score
                                  ? strength.color
                                  : "bg-overlay-strong"
                              }`}
                            />
                          ))}
                        </div>
                        <p className="text-[11px] text-muted-foreground">
                          {strength.label}
                        </p>
                      </div>
                    )}
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={registerForm.control}
                name="confirmPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel className="text-xs">
                      Confirm Password
                    </FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        placeholder="Re-enter your password"
                        autoComplete="new-password"
                        className="h-[42px] text-[13.5px]"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <Button
                type="submit"
                className="mt-1 h-[44px] w-full nyx-gradient-vivid text-[13px] font-medium shadow-[0_2px_12px_rgba(90,42,241,0.25)] hover:opacity-90 hover:shadow-[0_4px_20px_rgba(90,42,241,0.35)]"
                isLoading={registerMutation.isPending}
              >
                Create Account
              </Button>
            </form>
          </Form>

          {/* Footer */}
          <div className="mt-8 text-center text-[13px] text-muted-foreground">
            Already have an account?{" "}
            <button
              type="button"
              onClick={() => slideToPanel(0)}
              className="cursor-pointer font-medium text-foreground underline underline-offset-2 hover:text-nyx-secondary-400"
            >
              Sign in
            </button>
          </div>
        </div>
        </div>
        </div>
      )}
    </div>
  );
}
