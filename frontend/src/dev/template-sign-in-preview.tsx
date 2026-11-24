// Local Vite entry only. The hosted provider callback and device approval run
// on NyxID's registered origins. This tab polls for its own browser session.
import { useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { Button } from "@/components/ui/button";
import { useWebAuthDeviceLogin } from "@/hooks/use-auth-device";
import { useApplyTheme } from "@/hooks/use-theme";
import { appLanguageReady } from "@/i18n";
import { findSkillTemplate } from "@/lib/assistant/skill-templates";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import "@/app.css";

const provider = window.location.pathname.split("/").at(-1) ?? "";
const providerName = { google: "Google", apple: "Apple", github: "GitHub" }[
  provider
];
const query = new URLSearchParams(window.location.search);
const trusted = resolveTrustedAuthReturnTo(query.get("return_to") ?? undefined);
const target = trusted ? new URL(trusted) : null;
const template =
  target?.origin === window.location.origin && target.pathname === "/assistant"
    ? findSkillTemplate(target.searchParams.get("skill"))
    : undefined;

function providerUrl(verificationUrl: string) {
  const verification = new URL(verificationUrl);
  if (
    verification.origin !== "https://nyxid.ai" ||
    verification.pathname !== "/login/device"
  )
    return null;
  const url = new URL(
    `/api/v1/auth/social/${provider}`,
    "https://api.nyxid.ai",
  );
  url.searchParams.set("return_to", verification.href);
  const invite = query.get("invite_code");
  if (invite && invite.length <= 64)
    url.searchParams.set("invite_code", invite);
  return url.href;
}

export function PreviewSignIn() {
  useApplyTheme();
  const login = useWebAuthDeviceLogin();
  const started = useRef(false);
  const { start, phase } = login;
  useEffect(() => {
    if (started.current || !template || !providerName) return;
    started.current = true;
    start();
  }, [start]);
  useEffect(() => {
    if (phase === "success" && target && template)
      window.location.replace(target.href);
  }, [phase]);
  const link =
    login.request && phase === "pending"
      ? providerUrl(login.request.verification_uri_complete)
      : null;
  return (
    <main className="mx-auto max-w-xl space-y-5 px-6 py-12">
      <NyxidLogo />
      <h1 className="type-compact-page-title sm:type-page-title">
        {template?.title() ?? "NyxBot skill templates"}
      </h1>
      {template && providerName ? (
        <>
          <p>
            Sign in with {providerName} on NyxID, then approve this browser.
            Keep this tab open to return to your skill.
          </p>
          {link && (
            <div className="space-y-3">
              <Button asChild variant="primary">
                <a href={link} target="_blank" rel="noopener noreferrer">
                  Open {providerName} sign-in
                </a>
              </Button>
              <p className="type-secondary text-muted-foreground">
                Match this code on the approval screen:{" "}
                <strong>{login.request?.user_code}</strong>
              </p>
            </div>
          )}
          {["idle", "requesting", "restoring", "success"].includes(phase) && (
            <p role="status">
              {phase === "restoring" || phase === "success"
                ? "Opening NyxBot…"
                : "Preparing secure sign-in…"}
            </p>
          )}
          {login.error && <p role="alert">{login.error.message}</p>}
          {phase === "pending" && !link && (
            <p role="alert">
              NyxID returned an unavailable verification page. Start a new
              sign-in from the gallery.
            </p>
          )}
          {phase === "restricted" && (
            <p role="alert">
              Opening your personal NyxBot requires an account session. Start a
              new sign-in and choose account access.
            </p>
          )}
          {["expired", "denied", "error", "used"].includes(phase) && (
            <Button onClick={login.generateNew}>Start a new sign-in</Button>
          )}
          {phase === "restore_error" && (
            <Button onClick={login.retryProfile}>Retry account lookup</Button>
          )}
        </>
      ) : (
        <p>Choose a skill template to start sign-in.</p>
      )}
      <a className="block underline" href="/testing">
        Back to skill templates
      </a>
    </main>
  );
}

void appLanguageReady.then(() =>
  createRoot(document.getElementById("root")!).render(<PreviewSignIn />),
);
