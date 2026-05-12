import { useState, useRef } from "react";
import { useForm } from "react-hook-form";
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
import { toast } from "sonner";
import { usePublicConfig } from "@/hooks/use-public-config";

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
  if (score <= 4) return { score, label: "Fair", color: "bg-amber-400" };
  return { score, label: "Strong", color: "bg-success" };
}

const INVITE_PATTERN = /^NYX-[A-Z0-9]{8,}$/;

/** Map backend social-auth error keys to user-friendly messages. */
const SOCIAL_ERROR_MESSAGES: Record<string, string> = {
  social_auth_conflict:
    "This social account is already linked elsewhere. Please use your original sign-in method or contact support.",
  social_auth_no_email:
    "We couldn't retrieve an email address from your social account. Please ensure your email is public or use email/password sign-in.",
  social_auth_deactivated:
    "Your account has been deactivated. Please contact support for assistance.",
  social_auth_registration_closed:
    "No NyxID account found for this social login. Registration requires an invite code — please register with email and your invite code first, then sign in with your social account using the same email address to link it.",
  social_auth_failed: "Social sign-in failed. Please try again.",
  social_auth_exchange:
    "Social sign-in failed due to a temporary error. Please try again.",
};

/** Trusted origins for return_to redirect validation (open-redirect prevention). */
const BACKEND_URL = (
  (import.meta.env.VITE_BACKEND_URL as string | undefined) ??
  (import.meta.env.VITE_API_URL as string | undefined) ??
  ""
).replace(/\/+$/, "");

const FRONTEND_ORIGIN = window.location.origin;

// Social provider buttons for the register methods panel (full-width list style)
const REGISTER_PROVIDERS = [
  {
    id: "google",
    label: "Continue with Google",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24">
        <path
          d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z"
          fill="#4285F4"
        />
        <path
          d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
          fill="#34A853"
        />
        <path
          d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
          fill="#FBBC05"
        />
        <path
          d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
          fill="#EA4335"
        />
      </svg>
    ),
  },
  {
    id: "github",
    label: "Continue with GitHub",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
      </svg>
    ),
  },
  {
    id: "apple",
    label: "Continue with Apple",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor">
        <path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
      </svg>
    ),
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
  readonly initialInviteCode?: string;
}

export function AuthFlow({
  initialPanel = 0,
  returnTo,
  socialError,
  initialInviteCode,
}: AuthFlowProps) {
  const normalizedInitialInviteCode =
    initialInviteCode?.trim().toUpperCase() ?? "";
  const { data: publicConfig } = usePublicConfig();
  const inviteRequired = publicConfig?.invite_code_required ?? true;
  const emailAuthEnabled = publicConfig?.email_auth_enabled ?? false;

  const [panel, setPanel] = useState<AuthPanel>(
    initialPanel === 2 && !emailAuthEnabled ? 1 : initialPanel,
  );
  const [inviteError, setInviteError] = useState(false);
  const [fadeOpacity, setFadeOpacity] = useState(1);
  const fadingRef = useRef(false);
  const navigate = useNavigate();
  const isLogin = panel === 0;
  const showEmailForm = panel === 2;
  // Refs for focus after slide
  const loginEmailRef = useRef<HTMLInputElement>(null);
  const inviteInputRef = useRef<HTMLInputElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);

  // -- Forms --
  const loginForm = useForm<LoginFormData>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: "", password: "" },
  });

  const registerForm = useForm<RegisterFormData>({
    resolver: zodResolver(registerSchema),
    defaultValues: {
      inviteCode: normalizedInitialInviteCode,
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
  const inviteCode = registerForm.watch("inviteCode");
  const regPassword = registerForm.watch("password");
  const strength = getPasswordStrength(regPassword);
  const isInviteValid = INVITE_PATTERN.test(inviteCode.trim().toUpperCase());

  // Hide social providers whose backend credentials are not configured.
  // While publicConfig is loading, render none rather than flashing buttons
  // that may immediately disappear once the config arrives.
  const enabledProviders = REGISTER_PROVIDERS.filter(
    (p) => publicConfig?.social_providers.includes(p.id) ?? false,
  );
  const hasSocialProviders = enabledProviders.length > 0;

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
        if (initialInviteCode) nextParams.set("code", initialInviteCode);
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
      else if (target === 1) inviteInputRef.current?.focus();
      else nameInputRef.current?.focus();
    }, crossingLoginRegister ? FADE_MS * 2 + 50 : 350);
  }

  // -- Login submit --
  async function onLoginSubmit(data: LoginFormData) {
    try {
      const result = await loginMutation.mutateAsync(data);
      if (!result.mfaRequired) {
        if (
          returnTo &&
          (returnTo.startsWith(FRONTEND_ORIGIN + "/") ||
            returnTo.startsWith(BACKEND_URL + "/"))
        ) {
          window.location.assign(returnTo);
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
        invite_code: data.inviteCode,
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

  // -- Invite code gate for Step 2 --
  function requireInviteCode(): boolean {
    if (!inviteRequired) return true;
    const code = inviteCode.trim();
    if (!code) {
      setInviteError(true);
      inviteInputRef.current?.focus();
      return false;
    }
    setInviteError(false);
    return true;
  }

  // -- Register social login (passes invite code) --
  function handleRegisterSocialLogin(providerId: string) {
    if (!requireInviteCode()) return;
    const params = new URLSearchParams();
    if (returnTo) params.set("return_to", returnTo);
    const code = inviteCode.trim().toUpperCase();
    if (code) params.set("invite_code", code);
    const qs = params.toString();
    const url = `${window.location.origin}/api/v1/auth/social/${encodeURIComponent(providerId)}${qs ? `?${qs}` : ""}`;
    void openExternal(url);
  }

  return (
    <div
      className="-m-8 overflow-hidden rounded-xl"
      style={{
        opacity: fadeOpacity,
        transition: `opacity ${FADE_MS}ms ease-in-out`,
      }}
    >
      {isLogin ? (
        /* ================================================================
           Login View
           ================================================================ */
        <div className="px-7 pt-8 pb-7">
          <div className="mb-7 text-center">
            <h1 className="text-2xl font-bold tracking-tight nyx-gradient-text">
              Welcome back
            </h1>
            <p className="mt-1.5 text-[13px] text-muted-foreground">
              Sign in to your NyxID account
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

          {emailAuthEnabled && (
            <>
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
                    className="h-11 w-full nyx-gradient-vivid text-[12px] font-medium shadow-[0_2px_12px_rgba(90,42,241,0.25)] hover:opacity-90 hover:shadow-[0_4px_20px_rgba(90,42,241,0.35)]"
                    isLoading={loginMutation.isPending}
                  >
                    Sign In
                  </Button>
                </form>
              </Form>

              {hasSocialProviders && (
                <div className="my-6 flex items-center gap-4">
                  <div className="h-px flex-1 bg-border" />
                  <span className="text-xs text-text-tertiary">or</span>
                  <div className="h-px flex-1 bg-border" />
                </div>
              )}
            </>
          )}

          <div className="flex flex-col gap-2">
            {enabledProviders.map((provider) => (
              <button
                key={provider.id}
                type="button"
                onClick={() => {
                  const params = new URLSearchParams();
                  if (returnTo) params.set("return_to", returnTo);
                  const qs = params.toString();
                  const url = `${window.location.origin}/api/v1/auth/social/${encodeURIComponent(provider.id)}${qs ? `?${qs}` : ""}`;
                  void openExternal(url);
                }}
                className="flex h-[46px] cursor-pointer items-center gap-3 rounded-lg border border-border bg-background px-4 text-[13.5px] font-medium text-foreground transition-colors duration-300 hover:border-border/80 hover:bg-white/[0.03] active:scale-[0.99]"
              >
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-white/[0.06]">
                  {provider.icon}
                </span>
                {provider.label}
                <span className="ml-auto text-[12px] text-muted-foreground">
                  &rsaquo;
                </span>
              </button>
            ))}
          </div>

          {/* Footer */}
          <div className="mt-6 border-t border-border pt-5 text-center text-[13px] text-muted-foreground">
            Don&apos;t have an account?{" "}
            <button
              type="button"
              onClick={() => slideToPanel(1)}
              className="cursor-pointer font-medium text-nyx-secondary-400 hover:text-nyx-300"
            >
              Create account
            </button>
          </div>
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
        <div className="w-1/2 shrink-0 px-7 pt-8 pb-7">
          <div className="mb-7 text-center">
            <h1 className="text-2xl font-bold tracking-tight nyx-gradient-text">
              Create your account
            </h1>
            <p className="mt-1.5 text-[13px] text-muted-foreground">
              Start securing your digital identity
            </p>
          </div>

          {/* Step 1: Invite Code (only when invite gate is enabled) */}
          {inviteRequired && (
          <Form {...registerForm}>
            <div className="relative mb-6 pl-9">
              <div className="absolute left-[11px] top-7 bottom-[-12px] w-px bg-gradient-to-b from-nyx-500/10 to-transparent" />
              <div
                className={`absolute left-0 top-0 flex h-6 w-6 items-center justify-center rounded-full border text-[11px] font-semibold transition-colors duration-300 ${
                  isInviteValid
                    ? "border-transparent nyx-gradient-vivid text-white"
                    : "border-nyx-500/15 bg-nyx-500/10 text-nyx-secondary-400"
                }`}
              >
                {isInviteValid ? (
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 16 16"
                    fill="none"
                  >
                    <path
                      d="M3 8.5L6.5 12L13 4"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                ) : (
                  "1"
                )}
              </div>
              <p className="mb-3 text-[13px] font-medium leading-6 text-muted-foreground">
                Enter your invite code
              </p>
              <FormField
                control={registerForm.control}
                name="inviteCode"
                render={({ field }) => (
                  <FormItem>
                    <FormControl>
                      <Input
                        placeholder="NYX-XXXXXXXX"
                        autoComplete="off"
                        spellCheck={false}
                        className={`h-12 font-mono text-base tracking-wider ${
                          isInviteValid
                            ? "border-success/40 shadow-[0_0_0_3px_rgba(52,211,153,0.08)]"
                            : ""
                        }`}
                        {...field}
                        ref={(el) => {
                          field.ref(el);
                          (
                            inviteInputRef as React.MutableRefObject<HTMLInputElement | null>
                          ).current = el;
                        }}
                        onChange={(e) => {
                          field.onChange(e.target.value.toUpperCase());
                          if (inviteError) setInviteError(false);
                        }}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {inviteError && (
                <p className="mt-2 text-[12px] font-medium text-destructive">
                  An invite code is required to use NyxID at this time.
                </p>
              )}
              <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
                NyxID is in closed beta.{" "}
                <a
                  href="https://discord.gg/QMvcs8UQBW"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="cursor-pointer font-medium text-nyx-secondary-400 hover:text-nyx-300"
                >
                  Join our Discord
                </a>{" "}
                to request an invite code.
              </p>
            </div>
          </Form>
          )}

          {/* Step 2: Choose Method (becomes Step 1 when invite not required) */}
          <div className={`relative ${inviteRequired ? "pl-9" : ""}`}>
            {inviteRequired && (
            <div className="absolute left-0 top-0 flex h-6 w-6 items-center justify-center rounded-full border border-nyx-500/15 bg-nyx-500/10 text-[11px] font-semibold text-nyx-secondary-400">
              2
            </div>
            )}
            <p className="mb-3 text-[13px] font-medium leading-6 text-muted-foreground">
              Choose how to sign up
            </p>

            <div className="flex flex-col gap-2">
              {enabledProviders.map((provider) => (
                <button
                  key={provider.id}
                  type="button"
                  onClick={() => handleRegisterSocialLogin(provider.id)}
                  className="flex h-[46px] cursor-pointer items-center gap-3 rounded-lg border border-border bg-background px-4 text-[13.5px] font-medium text-foreground transition-colors duration-300 hover:border-border/80 hover:bg-white/[0.03] active:scale-[0.99]"
                >
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-white/[0.06]">
                    {provider.icon}
                  </span>
                  {provider.label}
                  <span className="ml-auto text-[12px] text-muted-foreground">
                    &rsaquo;
                  </span>
                </button>
              ))}

              {emailAuthEnabled && (
                <button
                  type="button"
                  onClick={() => { if (requireInviteCode()) slideToPanel(2); }}
                  className="flex h-[46px] cursor-pointer items-center gap-3 rounded-lg border border-border bg-background px-4 text-[13.5px] font-medium text-foreground transition-colors duration-300 hover:border-border/80 hover:bg-white/[0.03] active:scale-[0.99]"
                >
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-nyx-500/10">
                    <svg
                      className="h-4 w-4 text-nyx-secondary-400"
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
                  </span>
                  Continue with Email
                  <span className="ml-auto text-[12px] text-muted-foreground">
                    &rsaquo;
                  </span>
                </button>
              )}
            </div>
          </div>

          {/* Footer */}
          <div className="mt-6 border-t border-border pt-5 text-center text-[13px] text-muted-foreground">
            Already have an account?{" "}
            <button
              type="button"
              onClick={() => slideToPanel(0)}
              className="cursor-pointer font-medium text-nyx-secondary-400 hover:text-nyx-300"
            >
              Sign in
            </button>
          </div>
        </div>

        {/* Register Panel 2 — Email Registration */}
        <div
          className="w-1/2 shrink-0 px-7 pt-8 pb-7"
          onKeyDown={(e) => {
            if (e.key === "Escape") slideToPanel(1);
          }}
        >
          {/* Header with back button */}
          <div className="mb-6 flex items-center gap-3">
            <button
              type="button"
              onClick={() => slideToPanel(1)}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-transparent text-muted-foreground transition-colors duration-300 hover:border-border/80 hover:bg-white/[0.03] hover:text-foreground"
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

          {/* Invite code mirror (only when invite gate is enabled) */}
          {inviteRequired && (
          <div className="mb-5 flex items-center gap-2.5 rounded-lg border border-nyx-500/10 bg-nyx-500/10 px-3.5 py-2.5">
            <svg
              className="h-3.5 w-3.5 shrink-0 text-nyx-secondary-400"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="3" y="7" width="10" height="7" rx="1.5" />
              <path d="M5 7V5a3 3 0 016 0v2" />
            </svg>
            <span className="font-mono text-[13px] font-medium tracking-wider text-nyx-secondary-400">
              {inviteCode.trim().toUpperCase() || "NYX-XXXXXXXX"}
            </span>
            <button
              type="button"
              onClick={() => slideToPanel(1)}
              className="ml-auto border-0 bg-transparent text-[11px] font-medium text-muted-foreground transition-colors duration-300 hover:text-foreground"
            >
              Edit
            </button>
          </div>
          )}

          {/* Email registration form */}
          <Form {...registerForm}>
            <form
              onSubmit={registerForm.handleSubmit(onRegisterSubmit)}
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
                                  : "bg-white/[0.06]"
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
                className="mt-0.5 h-11 w-full nyx-gradient-vivid text-[12px] font-medium shadow-[0_2px_12px_rgba(90,42,241,0.25)] hover:opacity-90 hover:shadow-[0_4px_20px_rgba(90,42,241,0.35)]"
                isLoading={registerMutation.isPending}
              >
                Create Account
              </Button>
            </form>
          </Form>

          {/* Footer */}
          <div className="mt-6 border-t border-border pt-5 text-center text-[13px] text-muted-foreground">
            Already have an account?{" "}
            <button
              type="button"
              onClick={() => slideToPanel(0)}
              className="cursor-pointer font-medium text-nyx-secondary-400 hover:text-nyx-300"
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
