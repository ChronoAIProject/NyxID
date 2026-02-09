import { useForm } from "react-hook-form"
import { zodResolver } from "@hookform/resolvers/zod"
import { useNavigate, Link } from "@tanstack/react-router"
import { registerSchema, type RegisterFormData } from "@/schemas/auth"
import { useRegister } from "@/hooks/use-auth"
import { ApiError } from "@/lib/api-client"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { SocialLoginButtons } from "@/components/auth/social-login-buttons"
import { Separator } from "@/components/ui/separator"
import { toast } from "sonner"

function getPasswordStrength(password: string): {
  score: number
  label: string
  color: string
} {
  let score = 0
  if (password.length >= 8) score += 1
  if (password.length >= 12) score += 1
  if (/[A-Z]/.test(password)) score += 1
  if (/[a-z]/.test(password)) score += 1
  if (/[0-9]/.test(password)) score += 1
  if (/[^A-Za-z0-9]/.test(password)) score += 1

  if (score <= 2) return { score, label: "Weak", color: "bg-destructive" }
  if (score <= 4) return { score, label: "Fair", color: "bg-amber-500" }
  return { score, label: "Strong", color: "bg-emerald-500" }
}

export function RegisterForm() {
  const navigate = useNavigate()
  const registerMutation = useRegister()

  const form = useForm<RegisterFormData>({
    resolver: zodResolver(registerSchema),
    defaultValues: {
      name: "",
      email: "",
      password: "",
      confirmPassword: "",
    },
  })

  const password = form.watch("password")
  const strength = getPasswordStrength(password)

  async function onSubmit(data: RegisterFormData) {
    try {
      const result = await registerMutation.mutateAsync({
        name: data.name,
        email: data.email,
        password: data.password,
      })
      toast.success(result.message || "Account created successfully")
      void navigate({ to: "/login" as string })
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message })
      } else {
        form.setError("root", {
          message: "An unexpected error occurred. Please try again.",
        })
      }
    }
  }

  return (
    <div className="space-y-6">
      <div className="space-y-2 text-center">
        <h1 className="text-2xl font-semibold tracking-tight">
          Create an account
        </h1>
        <p className="text-sm text-muted-foreground">
          Get started with NyxID
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
            name="name"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Name</FormLabel>
                <FormControl>
                  <Input
                    placeholder="Your name"
                    autoComplete="name"
                    {...field}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />

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
                <FormLabel>Password</FormLabel>
                <FormControl>
                  <Input
                    type="password"
                    placeholder="Create a password"
                    autoComplete="new-password"
                    {...field}
                  />
                </FormControl>
                {password.length > 0 && (
                  <div className="space-y-1" aria-live="polite">
                    <div className="flex gap-1" role="progressbar" aria-valuenow={strength.score} aria-valuemin={0} aria-valuemax={6} aria-label={`Password strength: ${strength.label}`}>
                      {Array.from({ length: 6 }).map((_, i) => (
                        <div
                          key={`strength-${String(i)}`}
                          className={`h-1 flex-1 rounded-full ${
                            i < strength.score
                              ? strength.color
                              : "bg-muted"
                          }`}
                        />
                      ))}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {strength.label}
                    </p>
                  </div>
                )}
                <FormMessage />
              </FormItem>
            )}
          />

          <FormField
            control={form.control}
            name="confirmPassword"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Confirm Password</FormLabel>
                <FormControl>
                  <Input
                    type="password"
                    placeholder="Confirm your password"
                    autoComplete="new-password"
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
            isLoading={registerMutation.isPending}
          >
            Create account
          </Button>
        </form>
      </Form>

      <p className="text-center text-sm text-muted-foreground">
        Already have an account?{" "}
        <Link to="/login" className="text-primary hover:underline">
          Sign in
        </Link>
      </p>
    </div>
  )
}
