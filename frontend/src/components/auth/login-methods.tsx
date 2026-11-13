import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState, type ReactNode } from "react";
import { usePublicConfig } from "@/hooks/use-public-config";
import type { WebAuthDeviceTransport } from "@/hooks/use-auth-device";
import { Button } from "@/components/ui/button";
import { AUTH_PROVIDER_ICONS } from "./provider-icons";
import { LoginProviderRow } from "./login-provider-row";
import { WebDeviceLogin } from "./web-device-login";

export type LoginProvider = keyof typeof AUTH_PROVIDER_ICONS;
const PROVIDERS = [
  { id: "google", name: "Google" },
  { id: "github", name: "GitHub" },
  { id: "apple", name: "Apple" },
] as const;

export function LoginMethods({
  onSocial,
  onSignUp,
  returnTo,
  disabled = false,
  enableEmailLogin = false,
  appTransport,
  onAppVerified,
  onAppOpenChange,
  children,
}: {
  onSocial: (provider: LoginProvider) => void;
  onSignUp?: () => void;
  returnTo?: string;
  disabled?: boolean;
  /** Display override for existing-account login; never enables registration. */
  enableEmailLogin?: boolean;
  appTransport?: WebAuthDeviceTransport;
  onAppVerified?: () => void;
  onAppOpenChange?: (open: boolean) => void;
  children?: ReactNode;
}) {
  useUiTranslation();
  const { data: config, isPending, isError, refetch } = usePublicConfig();
  const [appOpen, setAppOpen] = useState(false);
  return (
    <>
      <div className={appOpen ? undefined : "flex flex-col gap-2.5"}>
        {!appOpen && isPending && (
          <p role="status" className="text-14 unthemed:text-12 text-muted-foreground">{translateUi("Loading sign-in methods...")}</p>
        )}
        {!appOpen && isError && (
          <div className="space-y-2">
            <p role="alert" className="text-14 unthemed:text-12 text-destructive">{translateUi("Could not load sign-in methods.")}</p>
            <Button
              type="button"
              disabled={disabled}
              onClick={() => void refetch()}
            >{translateUi("Try again")}</Button>
          </div>
        )}
        {!appOpen &&
          PROVIDERS.filter(({ id }) =>
            config?.social_providers?.includes(id),
          ).map(({ id, name }) => (
            <LoginProviderRow
              key={id}
              icon={AUTH_PROVIDER_ICONS[id]}
              label={translateUi("Continue with {{value1}}", { value1: name })}
              disabled={disabled}
              onClick={() => onSocial(id)}
            />
          ))}
        <WebDeviceLogin
          returnTo={returnTo}
          isOpen={appOpen}
          onOpenChange={(open) => {
            setAppOpen(open);
            onAppOpenChange?.(open);
          }}
          disabled={disabled}
          transport={appTransport}
          onVerified={onAppVerified}
        />
      </div>
      {!appOpen && (config?.email_auth_enabled || enableEmailLogin) && (
        <>
          <div className="my-6 flex items-center gap-4">
            <div className="h-px flex-1 bg-border" />
            <span className="text-13 text-text-tertiary">{translateUi("or")}</span>
            <div className="h-px flex-1 bg-border" />
          </div>
          {children}
        </>
      )}
      {!appOpen && (
        <div className="mt-8 text-center text-15 unthemed:text-13 text-muted-foreground">{translateUi("Don't have an account?")}{" "}
          {onSignUp ? (
            <button
              type="button"
              disabled={disabled}
              onClick={onSignUp}
              className="cursor-pointer font-medium text-foreground underline underline-offset-2 hover:text-nyx-secondary-400"
            >{translateUi("Sign up")}</button>
          ) : (
            <a
              href={`/register${returnTo ? `?${new URLSearchParams({ return_to: returnTo })}` : ""}`}
              className="font-medium text-foreground underline underline-offset-2 hover:text-nyx-secondary-400"
            >{translateUi("Sign up")}</a>
          )}
        </div>
      )}
    </>
  );
}
