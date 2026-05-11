import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";

// ── Last updated date (update on each revision) ──
const EFFECTIVE_DATE = "2026-05-11";

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
      <div className="w-full max-w-[980px] space-y-8">
        {/* ── Header ── */}
        <div className="flex flex-col items-center gap-4">
          <Link
            to="/"
            className="flex items-center transition-opacity hover:opacity-80"
          >
            <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-8 w-auto" />
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
              NyxID (&quot;we&quot;, &quot;our&quot;, or &quot;the
              Service&quot;) is an identity and access management platform. This
              Privacy Policy explains how we collect, use, store, and protect
              your personal information when you use our application and
              services.
            </p>
            <p>
              By using NyxID, you agree to the collection and use of information
              in accordance with this policy.
            </p>
          </Section>

          <Section title="2. Information We Collect">
            <p className="font-medium text-foreground">Account Information</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Email address (for registration and communication)</li>
              <li>Display name (optional, user-provided)</li>
              <li>
                Password (stored as a salted Argon2 hash, never in plaintext)
              </li>
            </ul>

            <p className="mt-3 font-medium text-foreground">
              Authentication Data
            </p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Session tokens and refresh tokens</li>
              <li>
                Multi-factor authentication (MFA) secrets (encrypted at rest)
              </li>
              <li>
                OAuth provider tokens when you connect third-party accounts
              </li>
            </ul>

            <p className="mt-3 font-medium text-foreground">Technical Data</p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>
                IP address and approximate geolocation (for security and audit)
              </li>
              <li>User-agent string and device type</li>
              <li>Timestamps of login events and API requests</li>
            </ul>
            <p className="mt-2 text-muted-foreground text-sm">
              These are received by our servers as part of normal HTTPS
              requests. The web application does not collect, store, or share
              this technical metadata beyond what the server needs for security
              and audit.
            </p>
          </Section>

          <Section title="3. How We Use Your Information">
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>Authenticate your identity and manage your sessions</li>
              <li>Provide single sign-on (SSO) to connected services</li>
              <li>
                Enforce security policies (rate limiting, anomaly detection)
              </li>
              <li>Send transactional emails (verification, password reset)</li>
              <li>Generate audit logs for administrative compliance</li>
              <li>Improve and maintain the Service</li>
            </ul>
          </Section>

          <Section title="4. Data Storage & Security">
            <p>
              All data is stored in encrypted MongoDB databases. Sensitive
              fields (OAuth tokens, MFA secrets, API credentials) are encrypted
              with AES-256 at the application layer. Passwords use Argon2id with
              per-user salts.
            </p>
            <p>
              All communications between the app and our servers use TLS 1.2+.
              JWT tokens are signed with RSA-256 keys rotated periodically.
            </p>
          </Section>

          <Section title="5. Data Sharing and Sub-processors">
            <p>
              We do <strong className="text-foreground">not</strong> sell, rent,
              or trade your personal information. We share data only in the
              following cases:
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
                <strong className="text-foreground">Security:</strong> to
                prevent fraud or protect the rights and safety of our users
              </li>
            </ul>

            <p className="mt-3 font-medium text-foreground">
              Sub-processors and service providers
            </p>
            <p>
              NyxID engages the following sub-processors to deliver parts of
              the Service. We have data processing agreements in place with
              each that require them to apply appropriate security measures
              and to process personal data only on our instructions:
            </p>
            <ul className="list-inside list-disc space-y-1 pl-2">
              <li>
                <strong className="text-foreground">PostHog Inc.</strong>{" "}
                (United States) — opt-in product analytics. Data category:
                anonymous usage events keyed to your NyxID account UUID.
                Processing region: US.
              </li>
              <li>
                <strong className="text-foreground">
                  Firebase Cloud Messaging
                </strong>{" "}
                by Google LLC (United States) — Android push-notification
                delivery, where push is enabled. Data category: device push
                token.
              </li>
              <li>
                <strong className="text-foreground">
                  Apple Push Notification service
                </strong>{" "}
                by Apple Inc. (United States) — iOS push-notification
                delivery, where push is enabled. Data category: device push
                token.
              </li>
            </ul>
            <p>
              NyxID also engages cloud infrastructure providers (for hosting
              and database services) and a transactional email service
              provider (for verification emails, password resets, and
              security notices). A current copy of our service-provider
              register and executed data processing agreements is available
              on request to{" "}
              <a
                href="mailto:contact@chrono-ai.fun"
                className="text-violet-400 underline decoration-violet-400/40 underline-offset-4 transition-colors hover:text-violet-300"
              >
                contact@chrono-ai.fun
              </a>
              . This list may be updated as the Service evolves; material
              changes will be reflected in this Privacy Policy with a revised
              effective date.
            </p>
            <p className="text-text-tertiary text-xs">
              Note: third-party platforms you connect yourself — including
              messaging-platform integrations (Telegram, Lark / Feishu,
              Discord, OpenClaw), Channel Bots you register, OAuth providers
              you use for social login (Google, GitHub, Apple), and any
              third-party APIs you call via the Credential Proxy — are{" "}
              <em>not</em> sub-processors of NyxID. They are independent
              services governed by their own terms and privacy policies.
            </p>
          </Section>

          <Section title="6. Data Retention">
            <p>
              Account data is retained for the lifetime of your account. When
              you delete your account, all personal data is permanently removed
              within 30 days. Audit logs may be retained for up to 90 days for
              security compliance before automatic purging.
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
              These actions are available through the Settings page in your
              NyxID dashboard, or by contacting us directly.
            </p>
          </Section>

          <Section title="8. Cookies, Local Storage, and Analytics">
            <p>
              NyxID uses HTTP-only secure cookies for session management and
              browser local storage to persist authentication state.
            </p>
            <p>
              <strong>Telemetry (opt-in).</strong> When you explicitly allow it
              via the consent banner on your first visit (or the toggle in
              Settings), NyxID collects anonymous usage events (pageviews,
              clicks, uncaught errors) through a third-party analytics
              provider (PostHog, US region). No credentials, form content,
              tokens, or the body of any request you make are ever captured.
              Sensitive URL segments (reset tokens, OAuth callback codes,
              approval IDs) are dropped at the egress layer before any event
              leaves your browser.
            </p>
            <p>
              <strong>EU→US transfer basis.</strong> PostHog Inc. is
              established in the United States, which does not benefit from
              an EU Commission adequacy decision at the time of this Privacy
              Policy's effective date. Where you are located in the European
              Economic Area, the United Kingdom, or another jurisdiction
              subject to cross-border transfer restrictions, your opt-in
              telemetry data is transferred to PostHog Inc. under the
              Standard Contractual Clauses (Module 2:
              Controller-to-Processor) approved by the European Commission
              in Implementing Decision (EU) 2021/914 of 4 June 2021,
              supplemented by encryption in transit (TLS 1.2 or higher),
              encryption at rest, scoped access controls, and the
              egress-scrubbing safeguards described above. A copy of the
              executed Standard Contractual Clauses is available on request
              to{" "}
              <a
                href="mailto:contact@chrono-ai.fun"
                className="text-violet-400 underline decoration-violet-400/40 underline-offset-4 transition-colors hover:text-violet-300"
              >
                contact@chrono-ai.fun
              </a>
              .
            </p>
            <p>
              Events are keyed to your NyxID account UUID after you sign in,
              allowing us to understand how our product is used in aggregate
              without requiring your name or email. Raw events are retained
              for 90 days; aggregated metrics may be retained longer. If you
              delete your NyxID account, the backend enqueues a matching
              delete request to the analytics provider so that your event
              history is removed.
            </p>
            <p>
              You can change your telemetry choice at any time from the
              Settings page. We honor the browser Do-Not-Track signal.
            </p>
            <p>
              <strong>Per-device scope.</strong> Your telemetry choice is
              stored on this browser and does not sync across the web
              dashboard, mobile app, and CLI. Each surface manages its own
              telemetry setting — the CLI uses{" "}
              <code>nyxid telemetry enable|disable</code> or the{" "}
              <code>DO_NOT_TRACK=1</code> environment variable, and the
              mobile app exposes a matching toggle in its own Settings
              screen.
            </p>
          </Section>

          <Section title="9. Children's Privacy">
            <p>
              NyxID is not intended for users under the age of 13. We do not
              knowingly collect personal information from children. If you
              believe a child has provided us with personal data, please contact
              us for immediate removal.
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
              contact@chrono-ai.fun
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
