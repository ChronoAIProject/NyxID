import { RegisterForm } from "@/components/auth/register-form";

export function RegisterPage() {
  const returnTo =
    new URLSearchParams(window.location.search).get("return_to") ?? undefined;

  return <RegisterForm returnTo={returnTo} />;
}
