// Display order is part of the sign-in screen contract, independent of server order.
export const socialProviders = [
  { id: "google", name: "Google", signInLabel: "Continue with Google" },
  { id: "github", name: "GitHub", signInLabel: "Continue with GitHub" },
  { id: "apple", name: "Apple", signInLabel: "Continue with Apple" },
] as const;

export type SocialProvider = (typeof socialProviders)[number]["id"];

export function isSocialProvider(value: string | null): value is SocialProvider {
  return socialProviders.some(({ id }) => id === value);
}
