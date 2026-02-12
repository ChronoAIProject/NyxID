import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate, Link } from "@tanstack/react-router";
import { loginSchema, type LoginFormData } from "@/schemas/auth";
import { useLogin } from "@/hooks/use-auth";
import { ApiError } from "@/lib/api-client";
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
import { SocialLoginButtons } from "@/components/auth/social-login-buttons";
import { Separator } from "@/components/ui/separator";

interface LoginFormProps {
  readonly returnTo?: string;
}

/** Trusted origins for return_to redirect validation (open-redirect prevention). */
const BACKEND_URL = (
  import.meta.env.VITE_BACKEND_URL ?? "http://localhost:3001"
).replace(/\/+$/, "");

/** Same-origin OAuth redirects go through the frontend nginx proxy. */
const FRONTEND_ORIGIN = window.location.origin;

export function LoginForm({ returnTo }: LoginFormProps) {
  const navigate = useNavigate();
  const loginMutation = useLogin();

  const form = useForm<LoginFormData>({
    resolver: zodResolver(loginSchema),
    defaultValues: {
      email: "",
      password: "",
    },
  });

  async function onSubmit(data: LoginFormData) {
    try {
      const result = await loginMutation.mutateAsync(data);
      if (!result.mfaRequired) {
        // If return_to was provided (OAuth browser flow), redirect back to the
        // authorize endpoint so it can issue the authorization code.
        // Accept same-origin URLs (proxied through frontend nginx) or the
        // explicit backend URL. Reject anything else to prevent open-redirect.
        if (
          returnTo &&
          (returnTo.startsWith(FRONTEND_ORIGIN + "/") ||
            returnTo.startsWith(BACKEND_URL + "/"))
        ) {
          window.location.assign(returnTo);
          return;
        }
        void navigate({ to: "/" as string });
      }
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        form.setError("root", {
          message: "An unexpected error occurred. Please try again.",
        });
      }
    }
  }

  return (
    <div className="space-y-6">
      <div className="space-y-2 text-center">
        <h1 className="text-2xl font-semibold tracking-tight">Welcome back</h1>
        <p className="text-sm text-muted-foreground">
          Sign in to your NyxID account
        </p>
      </div>

      <SocialLoginButtons />

      <div className="relative">
        <div className="absolute inset-0 flex items-center">
          <Separator className="w-full" />
        </div>
        <div className="relative flex justify-center text-xs uppercase">
          <span className="bg-card px-2 text-muted-foreground">
            Or continue with
          </span>
        </div>
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
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />

          <FormField
            control={form.control}
            name="password"
            render={({ field }) => (
              <FormItem>
                <div className="flex items-center justify-between">
                  <FormLabel>Password</FormLabel>
                  <Link
                    to={"/forgot-password" as string}
                    className="text-xs text-muted-foreground hover:text-primary"
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
            className="w-full"
            isLoading={loginMutation.isPending}
          >
            Sign in
          </Button>
        </form>
      </Form>

      <p className="text-center text-sm text-muted-foreground">
        {"Don't have an account? "}
        <Link to="/register" className="text-primary hover:underline">
          Sign up
        </Link>
      </p>
    </div>
  );
}
