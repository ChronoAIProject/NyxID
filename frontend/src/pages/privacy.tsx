import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";

import { useLogoHref } from "@/hooks/use-logo-href";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";

// ── Last updated date (update on each revision) ──
const EFFECTIVE_DATE = "2026-02-25";

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      <h2 className="text-base font-semibold text-foreground">{title}</h2>
      <div className="space-y-2 text-12 leading-relaxed text-muted-foreground">
        {children}
      </div>
    </section>
  );
}

export function PrivacyPage() {
  const { t } = useUiTranslation();
  useApplyTheme();
  const logoHref = useLogoHref();

  return (
    <div
      className="flex min-h-dvh flex-col items-center bg-background px-4 py-8"
      style={{
        paddingTop: "max(2rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className="w-full max-w-[680px] space-y-8">
        {/* ── Header ── */}
        <div className="flex flex-col items-center gap-4">
          <Link
            to={logoHref}
            className="flex items-center transition-opacity hover:opacity-80"
          >
            <NyxidLogo className="h-8 w-auto" />
          </Link>
          <h1 className="text-2xl font-bold text-foreground">{t("Privacy Policy")}</h1>
          <p className="text-xs text-text-tertiary">{t("Effective date: ")}{EFFECTIVE_DATE}
          </p>
        </div>

        {/* ── Body ── */}
        <div className="space-y-6 rounded-xl border border-border bg-card p-6 sm:p-8">
          <Section title={t("1. Introduction")}>
            <p>{t("NyxID (\"we\", \"our\", or \"the Service\") is an identity and access management platform. This Privacy Policy explains how we collect, use, store, and protect your personal information when you use our application and services.")}</p>
            <p>{t("By using NyxID, you agree to the collection and use of information in accordance with this policy.")}</p>
          </Section>

          <Section title={t("2. Information We Collect")}>
            <p className="font-medium text-foreground">{t("Account Information")}</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>{t("Email address (for registration and communication)")}</li>
              <li>{t("Display name (optional, user-provided)")}</li>
              <li>{t("Password (stored as a salted Argon2 hash, never in plaintext)")}</li>
            </ul>

            <p className="mt-3 font-medium text-foreground">{t("Authentication Data")}</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>{t("Session tokens and refresh tokens")}</li>
              <li>{t("Multi-factor authentication (MFA) secrets (encrypted at rest)")}</li>
              <li>{t("OAuth provider tokens when you connect third-party accounts")}</li>
            </ul>

            <p className="mt-3 font-medium text-foreground">{t("Technical Data")}</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>{t("IP address and approximate geolocation (for security and audit)")}</li>
              <li>{t("User-agent string and device type")}</li>
              <li>{t("Timestamps of login events and API requests")}</li>
            </ul>
            <p className="mt-2 text-muted-foreground text-12">{t("These are received by our servers as part of normal HTTPS requests. The web application does not collect, store, or share this technical metadata beyond what the server needs for security and audit.")}</p>
          </Section>

          <Section title={t("3. How We Use Your Information")}>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>{t("Authenticate your identity and manage your sessions")}</li>
              <li>{t("Provide single sign-on (SSO) to connected services")}</li>
              <li>{t("Enforce security policies (rate limiting, anomaly detection)")}</li>
              <li>{t("Send transactional emails (verification, password reset)")}</li>
              <li>{t("Generate audit logs for administrative compliance")}</li>
              <li>{t("Improve and maintain the Service")}</li>
            </ul>
          </Section>

          <Section title={t("4. Data Storage & Security")}>
            <p>{t("All data is stored in encrypted MongoDB databases. Sensitive fields (OAuth tokens, MFA secrets, API credentials) are encrypted with AES-256 at the application layer. Passwords use Argon2id with per-user salts.")}</p>
            <p>{t("All communications between the app and our servers use TLS 1.2+. JWT tokens are signed with RSA-256 keys rotated periodically.")}</p>
          </Section>

          <Section title={t("5. Data Sharing")}>
            <p>{t("We do ")}<strong className="text-foreground">{t("not")}</strong>{t(" sell, rent, or trade your personal information. We share data only in the following cases:")}</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>
                <strong className="text-foreground">{t("With your consent:")}</strong>{" "}{t("when you authorize a third-party service via OAuth/OIDC")}</li>
              <li>
                <strong className="text-foreground">{t("Legal obligations:")}</strong>{" "}{t("when required by law, regulation, or valid legal process")}</li>
              <li>
                <strong className="text-foreground">{t("Security:")}</strong>{t(" to prevent fraud or protect the rights and safety of our users")}</li>
            </ul>
          </Section>

          <Section title={t("6. Data Retention")}>
            <p>{t("Account data is retained for the lifetime of your account. When you delete your account, all personal data is permanently removed within 30 days. Audit logs may be retained for up to 90 days for security compliance before automatic purging.")}</p>
          </Section>

          <Section title={t("7. Your Rights")}>
            <p>{t("You have the right to:")}</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>{t("Access and export your personal data")}</li>
              <li>{t("Correct inaccurate information in your profile")}</li>
              <li>{t("Delete your account and associated data")}</li>
              <li>{t("Revoke consent for third-party service connections")}</li>
              <li>{t("Opt out of non-essential communications")}</li>
            </ul>
            <p>{t("These actions are available through the Settings page in your NyxID dashboard, or by contacting us directly.")}</p>
          </Section>

          <Section title={t("8. Cookies, Local Storage, and Analytics")}>
            <p>{t("NyxID uses HTTP-only secure cookies for session management and browser local storage to persist authentication state.")}</p>
            <p>
              <strong>{t("Telemetry (opt-in).")}</strong>{t(" When you explicitly allow it via the consent banner on your first visit (or the toggle in Settings), NyxID collects anonymous usage events (pageviews, clicks, uncaught errors) through a third-party analytics provider (PostHog, US region). No credentials, form content, tokens, or the body of any request you make are ever captured. Sensitive URL segments (reset tokens, OAuth callback codes, approval IDs) are dropped at the egress layer before any event leaves your browser.")}{/* TODO(legal): document EU→US transfer basis (SCCs / adequacy)
                  here before broader EU launch. Tracked in
                  docs/TELEMETRY_CONSENT_FIX.md §9.2. */}
            </p>
            <p>{t("Events are keyed to your NyxID account UUID after you sign in, allowing us to understand how our product is used in aggregate without requiring your name or email. Raw events are retained for 90 days; aggregated metrics may be retained longer. If you delete your NyxID account, the backend enqueues a matching delete request to the analytics provider so that your event history is removed.")}</p>
            <p>{t("You can change your telemetry choice at any time from the Settings page. We honor the browser Do-Not-Track signal.")}</p>
            <p>
              <strong>{t("Per-device scope.")}</strong>{t(" Your telemetry choice is stored on this browser and does not sync across the web dashboard, mobile app, and CLI. Each surface manages its own telemetry setting — the CLI uses")}{" "}
              <code>nyxid telemetry enable|disable</code>{t(" or the")}{" "}
              <code>DO_NOT_TRACK=1</code>{t(" environment variable, and the mobile app exposes a matching toggle in its own Settings screen.")}</p>
          </Section>

          <Section title={t("9. Children's Privacy")}>
            <p>{t("NyxID is not intended for users under the age of 13. We do not knowingly collect personal information from children. If you believe a child has provided us with personal data, please contact us for immediate removal.")}</p>
          </Section>

          <Section title={t("10. Changes to This Policy")}>
            <p>{t("We may update this Privacy Policy from time to time. Changes will be posted on this page with an updated effective date. Continued use of the Service after changes constitutes acceptance of the revised policy.")}</p>
          </Section>

          <Section title={t("11. Contact Us")}>
            <p>{t("If you have any questions about this Privacy Policy or your data, please contact us at:")}</p>
            <p className="text-xs text-foreground">{t("privacy@nyxid.com")}</p>
          </Section>
        </div>

        {/* ── Footer ── */}
        <div className="flex justify-center">
          <Link
            to="/"
            className="flex items-center gap-1.5 text-xs text-nyx-secondary-400 transition-colors duration-300 hover:text-nyx-300"
          >
            <ArrowLeft className="h-3 w-3" />{t("Back to NyxID")}</Link>
        </div>
      </div>
    </div>
  );
}
