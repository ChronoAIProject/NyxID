import { Link } from "@tanstack/react-router";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";
import { ArrowLeft } from "lucide-react";

// ── Last updated date (update on each revision) ──
const EFFECTIVE_DATE = "2026-02-25";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <h2 className="text-base font-semibold text-foreground">{title}</h2>
      <div className="space-y-2 text-sm leading-relaxed text-muted-foreground">
        {children}
      </div>
    </section>
  );
}

export function PrivacyPage() {
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
            to="/"
            className="flex items-center gap-3 transition-opacity hover:opacity-80"
          >
            <PortalMarkLogo size={32} className="shrink-0" />
            <span className="logo-wordmark text-xl">NyxID</span>
          </Link>
          <h1 className="text-2xl font-bold text-foreground">Privacy Policy</h1>
          <p className="text-xs text-text-tertiary">
            Effective date: {EFFECTIVE_DATE}
          </p>
        </div>

        {/* ── Body ── */}
        <div className="space-y-6 rounded-xl border border-border bg-card p-6 sm:p-8">
          <Section title="1. Introduction">
            <p>
              NyxID (&quot;we&quot;, &quot;our&quot;, or &quot;the Service&quot;) is an identity and
              access management platform. This Privacy Policy explains how we collect,
              use, store, and protect your personal information when you use our
              application and services.
            </p>
            <p>
              By using NyxID, you agree to the collection and use of information in
              accordance with this policy.
            </p>
          </Section>

          <Section title="2. Information We Collect">
            <p className="font-medium text-foreground">Account Information</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Email address (for registration and communication)</li>
              <li>Display name (optional, user-provided)</li>
              <li>Password (stored as a salted Argon2 hash, never in plaintext)</li>
            </ul>

            <p className="mt-3 font-medium text-foreground">Authentication Data</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Session tokens and refresh tokens</li>
              <li>Multi-factor authentication (MFA) secrets (encrypted at rest)</li>
              <li>OAuth provider tokens when you connect third-party accounts</li>
            </ul>

            <p className="mt-3 font-medium text-foreground">Technical Data</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>IP address and approximate geolocation (for security and audit)</li>
              <li>User-agent string and device type</li>
              <li>Timestamps of login events and API requests</li>
            </ul>
          </Section>

          <Section title="3. How We Use Your Information">
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Authenticate your identity and manage your sessions</li>
              <li>Provide single sign-on (SSO) to connected services</li>
              <li>Enforce security policies (rate limiting, anomaly detection)</li>
              <li>Send transactional emails (verification, password reset)</li>
              <li>Generate audit logs for administrative compliance</li>
              <li>Improve and maintain the Service</li>
            </ul>
          </Section>

          <Section title="4. Data Storage & Security">
            <p>
              All data is stored in encrypted MongoDB databases. Sensitive fields
              (OAuth tokens, MFA secrets, API credentials) are encrypted with
              AES-256 at the application layer. Passwords use Argon2id with
              per-user salts.
            </p>
            <p>
              All communications between the app and our servers use TLS 1.2+.
              JWT tokens are signed with RSA-256 keys rotated periodically.
            </p>
          </Section>

          <Section title="5. Data Sharing">
            <p>
              We do <strong className="text-foreground">not</strong> sell, rent, or
              trade your personal information. We share data only in the following
              cases:
            </p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>
                <strong className="text-foreground">With your consent:</strong>{" "}
                when you authorize a third-party service via OAuth/OIDC
              </li>
              <li>
                <strong className="text-foreground">Legal obligations:</strong>{" "}
                when required by law, regulation, or valid legal process
              </li>
              <li>
                <strong className="text-foreground">Security:</strong>{" "}
                to prevent fraud or protect the rights and safety of our users
              </li>
            </ul>
          </Section>

          <Section title="6. Data Retention">
            <p>
              Account data is retained for the lifetime of your account. When you
              delete your account, all personal data is permanently removed within
              30 days. Audit logs may be retained for up to 90 days for security
              compliance before automatic purging.
            </p>
          </Section>

          <Section title="7. Your Rights">
            <p>You have the right to:</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Access and export your personal data</li>
              <li>Correct inaccurate information in your profile</li>
              <li>Delete your account and associated data</li>
              <li>Revoke consent for third-party service connections</li>
              <li>Opt out of non-essential communications</li>
            </ul>
            <p>
              These actions are available through the Settings page in your NyxID
              dashboard, or by contacting us directly.
            </p>
          </Section>

          <Section title="8. Cookies & Local Storage">
            <p>
              NyxID uses HTTP-only secure cookies for session management. We use
              browser local storage (via Capacitor Preferences on mobile) to
              persist authentication state. We do not use third-party tracking
              cookies or analytics services.
            </p>
          </Section>

          <Section title="9. Children&apos;s Privacy">
            <p>
              NyxID is not intended for users under the age of 13. We do not
              knowingly collect personal information from children. If you believe
              a child has provided us with personal data, please contact us for
              immediate removal.
            </p>
          </Section>

          <Section title="10. Changes to This Policy">
            <p>
              We may update this Privacy Policy from time to time. Changes will
              be posted on this page with an updated effective date. Continued
              use of the Service after changes constitutes acceptance of the
              revised policy.
            </p>
          </Section>

          <Section title="11. Contact Us">
            <p>
              If you have any questions about this Privacy Policy or your data,
              please contact us at:
            </p>
            <p className="font-mono text-xs text-foreground">
              privacy@nyxid.com
            </p>
          </Section>
        </div>

        {/* ── Footer ── */}
        <div className="flex justify-center">
          <Link
            to="/"
            className="flex items-center gap-1.5 text-xs text-violet-400 transition-colors hover:text-violet-300"
          >
            <ArrowLeft className="h-3 w-3" />
            Back to NyxID
          </Link>
        </div>
      </div>
    </div>
  );
}
