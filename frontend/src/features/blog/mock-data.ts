import type {
  BlogArticle,
  DirectusUser,
  Product,
  Series,
  Tag,
} from "./types";

// ──────────────────────── article bodies (GFM markdown) ────────────────────

const PUSH_APPROVALS_BODY = `The pitch for push approvals is a single sentence: when something risky is about to happen with your credentials, your phone asks first. The implementation, of course, is more than that. Here's a walk through one approval — from the agent reaching for a database to your thumb committing — and the choices we made about what crosses the wire and what doesn't.

## The problem with TOTP

Six-digit codes from an authenticator app solved one problem beautifully: they proved you possess a device. They solved nothing about *what you were authorising*. A code is a blank cheque — type it in, and whatever's on the other end of the login flow proceeds. This is fine when the only thing on the other end is "log into Gmail". It is not fine when the other end is "let \`claude-code\` write to \`db-prod-us-east-1\` for two hours."

> A code is a blank cheque. We wanted the cheque to show the amount before you signed it.

What you actually need to make a confident decision is everything **around** the request: who's asking, what they want, against which resource, for how long, and whether anything about this looks unusual. Not after you approve. Before.

## The shape of an approval

When a request hits the proxy and the matched policy says \`approval_mode = per_request\`, NyxID composes a payload for your phone. The payload is small on purpose. Here's what actually shows up:

- **Requested by** — the agent or service account making the call. Not a user-supplied label; the platform-bound identity from the API key.
- **Resource** — the downstream service slug and endpoint, in human terms. \`postgres://db-prod-us-east-1\`, not a UUID.
- **Operation** — coarse capability, not the SQL. \`READ\` / \`WRITE\` / \`DEPLOY\`.
- **Window** — how long the grant lives if you say yes. Always finite.

That's it. The push notification itself contains even less — only a request ID. Details are pulled by the phone over a mutually authenticated channel when you open the card. If the push payload leaks (someone screenshotting your lock screen, a misbehaving backup), there's nothing in it that helps an attacker.

## Anatomy of the request

Under the hood, the proxy emits an \`ApprovalRequest\` document, signs it, and pushes it to the device tokens registered for that user. The agent's HTTP request, meanwhile, is parked on the proxy's side — held in a per-request state machine with a countdown.

\`\`\`rust
pub async fn request_approval(
    db: &Database,
    actor: &AuthUser,
    target: &UserService,
    op: Operation,
    window: Duration,
) -> AppResult<ApprovalRequest> {
    // 1. Compose the minimal payload — what the user must see.
    let req = ApprovalRequest::new(actor, target, op, window);
    db.collection::<ApprovalRequest>("approval_requests")
      .insert_one(&req).await?;

    // 2. Push only the request_id. Details are fetched on tap.
    push_service::notify(actor.user_id, &req.id).await?;

    // 3. Audit before we wait.
    audit::log("approval.requested", &req).await?;
    Ok(req)
}
\`\`\`

Three things happen in order: the request lands in MongoDB, a push with only the request ID goes out, and the audit log gets a row. If any step fails the request never reaches your phone, and the agent gets a clean error code (\`1010 — approval pending\` transitions to \`1012 — approval failed\`).

### What the tap actually does

Tapping **Approve** mints a short-lived grant — a signed token bound to *this* request, *this* resource, and the window you saw on screen. The proxy's parked request is woken up, the grant is checked, and the call continues to the downstream service. Tapping **Deny** writes a denial row and returns \`1011 — approval denied\` to the agent. Either way, the round-trip is logged with attribution down to the API key.

## What we deliberately don't send

A surprising amount of the design lives in the things we kept out of the wire. The shortlist:

1. The actual API call body. The proxy holds it; your phone never sees it.
2. Any credentials at all. Tokens, keys, secrets — none of these ever reach the device.
3. Free-form labels supplied by the requesting agent. The agent identity is platform-bound and signed by the issuance flow.
4. Cross-tenant context. If your user belongs to two orgs, the approval is scoped to the org that owns the resource — not merged.

The thing we're protecting is the *integrity of the decision* — that what you see is what gets approved, and what you see is enough to decide. Everything outside that goal is overhead.

## Try it

If you're in the beta, \`nyxid service add --slug postgres-prod --approval per_request\` turns this on for any service you've registered. The mobile app reaches you within a second on most carriers; if your network is slow, the proxy will wait a configurable window (default 30s) before failing closed.
`;

const ENVELOPE_ENCRYPTION_BODY = `"Envelope encryption" sounds like marketing. It isn't — it's the only sane way to manage encryption keys at scale, and it's the structural choice that makes "we never see your tokens" technically true.

## What it actually means

Every secret in NyxID is wrapped with a **data encryption key (DEK)**. The DEK is itself wrapped with a **key encryption key (KEK)**. The KEK lives in a KMS — AWS KMS, GCP Cloud KMS, or a local file in dev. The DEK lives next to the ciphertext.

To decrypt:

1. Read the wrapped DEK.
2. Ask the KMS to unwrap it. (This is the only operation that touches the KMS.)
3. Use the DEK to decrypt the secret.
4. Throw the DEK away.

The KMS never sees the plaintext secret. Your application code never holds the KEK. Each secret has a different DEK. If a single ciphertext leaks, the blast radius is one secret — not the whole vault.

## Why we wrap the wrapper

The naive design is "use the KMS to encrypt the secret directly." This works until you discover three things:

- **Latency.** Every read becomes a network call to the KMS.
- **Cost.** KMS calls are billed per request. A high-throughput proxy makes this painful.
- **Size.** KMS payload limits are small (4 KB on AWS KMS). Real secrets — JSON config, multi-line PEM keys — exceed this often enough to matter.

Envelope encryption fixes all three. The KMS only handles short DEKs, the unwrapped DEK can be cached briefly under a strict TTL, and the actual ciphertext can be any size.

## The schema

Each encrypted column carries a small struct:

\`\`\`rust
pub struct EncryptedBlob {
    /// Versioned ciphertext (AES-256-GCM).
    pub ciphertext: Vec<u8>,
    /// Random 96-bit IV.
    pub nonce: [u8; 12],
    /// DEK wrapped by the KEK currently in use.
    pub wrapped_dek: Vec<u8>,
    /// Identifies which KEK to ask for.
    pub kek_id: String,
}
\`\`\`

The \`kek_id\` is the key. It lets us **rotate**: a new KEK gets a new id; old blobs still reference the old KEK; reads use whatever \`kek_id\` they have; writes use the latest. Migration is incremental and zero-downtime — re-encrypt a blob lazily on the next write.

## Switching providers

The architecture has a \`KeyProvider\` trait. Local file is one impl, AWS KMS is another, GCP Cloud KMS is a third. A "fallback" provider lets you migrate from one backend to another without re-encrypting everything up front: writes go to the new backend, reads check both.

We've never had to use it in anger. We're glad it's there.
`;

const OAUTH_DELEGATION_BODY = `OAuth 2.0 token exchange — RFC 8693 — is one of those specs that almost everybody references and almost nobody implements. The shape is: trade a token you have for a token you want, possibly with reduced scope, possibly on behalf of someone else.

We needed it for two reasons.

## Reason one: service accounts

A service account that wants to call \`provider-x\` on behalf of a particular user can't just impersonate them. We needed delegation: a token that says "this service account, acting for this user, with this subset of scopes, until this expiry."

Token exchange is the right primitive. The service account presents its own credentials *and* the user's subject identifier (or a token that vouches for the user), and the authorization server returns a delegated access token bound to both.

## Reason two: native social login

Mobile apps have an awkward problem with social login. The OAuth redirect flow makes sense in a browser; on a phone it requires shipping the user to Safari and back through deep links. The cleaner pattern: the app uses Google Sign-In natively, gets an ID token, and then *exchanges* that ID token at our authorization server for our own access/refresh tokens.

That's literally token exchange with \`subject_token_type=urn:ietf:params:oauth:token-type:id_token\`. The \`subject_token\` is the Google ID token. We verify the signature against Google's JWKS, mint our own tokens, and the mobile app never sees a redirect.

## What the audit log taught us

Once we wired token exchange in, audit log volume jumped. Not dramatically — but the shape of the data changed. Each delegated request now carried *two* identities: the actor (service account) and the subject (user). Our existing audit query patterns assumed one principal per row.

Three weeks of log dashboards and one schema migration later, we landed on:

- \`actor_id\` — the entity that authenticated.
- \`subject_id\` — the entity on whose behalf the action was taken (often equal to \`actor_id\` for non-delegated calls).
- \`delegation_chain\` — array, present only when token exchange was used.

The lesson, in retrospect, was that delegation is not an edge case. It's the common case for any system where automation acts on behalf of humans. Designing the audit log for one identity per request is the wrong default.
`;

const DESIGN_BODY = `Tailwind's \`violet-500\` is \`#8b5cf6\`. It is also the AI-assistant default — the colour every chatbot UI in the world snaps to when nobody pushes back. We did not want to look like every chatbot UI in the world.

## Six points warmer

We sampled forty purples around violet-500 and held each next to our existing greys. The shortlist had three:

- \`#8b5cf6\` — Tailwind violet-500.
- \`#9775fa\` — six points warmer, slightly less saturated.
- \`#a78bfa\` — Tailwind violet-400, much lighter.

\`#9775fa\` won. It still reads as "purple" at a glance, but it's noticeably less screamy. Side by side with the original it looks more deliberate. Side by side with copies of every AI chat UI on the internet, it looks like ours.

> Color is earned. Purple marks identity and interaction, nothing else.

## Where purple is allowed

The design system has a single rule: **purple appears on identity and interaction, nothing else.** Identity is the logo, the wordmark, the active nav state, the brand mark. Interaction is the focus ring, the pending-approval left border, the count badge.

It is not on every hover state. It is not the background of the page. It is not the colour of every button. The semantic colours — green for success, amber for attention, red for error — do the heavy lifting for status. Purple does the heavy lifting for *us*.

## Why the rule matters

Decoration without restraint feels like a Bootstrap demo from 2014: every button a colour, every gradient pulling for attention. The opposite — pure greyscale — feels like an enterprise wiki. The middle path is to have one accent and to use it like punctuation: rare enough that it means something, frequent enough that the eye finds it.

If we ever break the rule, the system stops working. So we don't.
`;

const AUDIT_POSTMORTEM_BODY = `At 04:17 on a Tuesday, the on-call phone went off. A customer was asking why their audit log had a hole.

The hole was sixty-three seconds long. Inside it: a deploy, a brief credential rotation, and twelve proxy requests that, by every other piece of telemetry, definitely happened. The audit log did not show them.

## What we found

The cause was banal: a worker thread in the audit pipeline had silently exited after an upstream dependency raised a recoverable error. The error was caught, logged at \`info\`, and the thread… didn't restart. The supervising task assumed the thread was healthy because it never returned an \`Err\`.

Three things fell into place to make this invisible:

1. The error was logged at \`info\`, below our default alert threshold.
2. The proxy continued to work — only the audit-write side was affected.
3. The audit log's downstream consumers (the customer dashboard, our own queries) showed gaps but didn't *alert* on them.

Each of those was reasonable in isolation. Together they meant a failure mode that survived for sixty-three seconds before it self-healed, and would have survived indefinitely if the customer hadn't checked.

## What we changed

The fix was a one-line restart loop. The interesting work was downstream:

- We added a continuity check: every minute, the audit pipeline writes a heartbeat row. The dashboard alerts if more than ninety seconds have passed without one.
- We promoted the worker-error log line to \`error\` and added an alert.
- We added a post-deploy verification step that compares proxy request counts to audit row counts for a five-minute window. If they diverge, the deploy is flagged.

## What we promised

"Every action is logged" is the kind of promise that decays without active maintenance. The post-mortem ended with a commitment: every quarter, run a planned outage of the audit pipeline in staging, and verify that *all* of the downstream alerts fire. So far we've run it three times. Twice we've found a regression we wouldn't have caught any other way.

The lesson is not "logging is hard." The lesson is that promises about logging require their own monitoring, because the system that's supposed to tell you something is broken cannot tell you when it itself is broken.
`;

// ──────────────────────── reference data ────────────────────

const NYXID_PRODUCT: Product = {
  id: "product-nyxid",
  name: "NyxID",
  site_url: "https://nyxid.io",
  site_github_repo: "ChronoAIProject/NyxID",
  site_dispatch_event_type: "blog-publish",
  content_path: "nyxid/",
};

const TAGS = {
  approvals: {
    id: "tag-approvals",
    slug: "push-approvals",
    name: "push-approvals",
  },
  mobile: { id: "tag-mobile", slug: "mobile", name: "mobile" },
  security: { id: "tag-security", slug: "security", name: "security" },
  audit: { id: "tag-audit", slug: "audit", name: "audit" },
  product: { id: "tag-product", slug: "product", name: "product" },
  encryption: {
    id: "tag-encryption",
    slug: "encryption",
    name: "encryption",
  },
  kms: { id: "tag-kms", slug: "kms", name: "kms" },
  oauth: { id: "tag-oauth", slug: "oauth", name: "oauth" },
  delegation: {
    id: "tag-delegation",
    slug: "delegation",
    name: "delegation",
  },
  design: { id: "tag-design", slug: "design", name: "design" },
  color: { id: "tag-color", slug: "color", name: "color" },
  postmortem: {
    id: "tag-postmortem",
    slug: "postmortem",
    name: "postmortem",
  },
} as const satisfies Record<string, Tag>;

const SERIES_FOUNDATIONS: Series = {
  id: "series-foundations",
  slug: "foundations",
  name: "Foundations",
  description: "How NyxID is built, one piece at a time.",
};

const PRIYA: DirectusUser = {
  id: "user-priya",
  first_name: "Priya",
  last_name: "Ramesh",
  email: "priya@nyxid.dev",
  title: "Product",
  description:
    "Product at NyxID. Previously identity at a healthtech you haven't heard of, and an embedded systems lab where she got very tired of TOTP.",
};

const MARCUS: DirectusUser = {
  id: "user-marcus",
  first_name: "Marcus",
  last_name: "Kell",
  email: "marcus@nyxid.dev",
  title: "Security Engineering",
  description:
    "Security engineering at NyxID. Spends most of his time thinking about key rotation and the weird shapes that envelope encryption forces on database schemas.",
};

const CALVIN: DirectusUser = {
  id: "user-calvin",
  first_name: "Calvin",
  last_name: "Tan",
  email: "calvin@nyxid.dev",
  title: "Founder",
  description:
    "Founder of NyxID. Writes about identity, OAuth grants nobody implements, and the ergonomics of giving agents the right kind of access.",
};

const JAMIE: DirectusUser = {
  id: "user-jamie",
  first_name: "Jamie",
  last_name: "Liu",
  email: "jamie@nyxid.dev",
  title: "Design",
  description:
    "Design at NyxID. Cares about hierarchy, contrast, and not abusing accent colours.",
};

// Stable Unsplash photo ID with rendition params so the CDN resizes per page.
function unsplash(id: string): string {
  return `https://images.unsplash.com/${id}?auto=format&fit=crop&w=1600&q=80`;
}

// ──────────────────────── articles ────────────────────

export const MOCK_ARTICLES: readonly BlogArticle[] = [
  {
    id: "9a0f6a4d-6a3a-4f8b-9b6c-001000000001",
    product: NYXID_PRODUCT,
    slug: "push-approvals-walkthrough",
    title: "Push approvals: a 90-second walkthrough",
    description:
      "What actually happens between the request and the tap. We trace one approval from claude-code reaching for a production database, to your phone lighting up, to the request continuing through.",
    body: PUSH_APPROVALS_BODY,
    tags: [TAGS.approvals, TAGS.mobile, TAGS.product, TAGS.security],
    series: SERIES_FOUNDATIONS,
    author: PRIYA,
    hero_image: {
      id: "file-push-approvals",
      filename_disk: "push-approvals.jpg",
      url: unsplash("photo-1534796636912-3b95b3ab5986"),
      width: 1600,
      height: 1067,
      alt: "A bright moon against a deep night sky.",
    },
    published_at: "2026-04-22T09:00:00Z",
    status: "published",
    content_commit_sha: "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0",
    content_url:
      "https://github.com/ChronoAIProject/NyxID-content/blob/main/nyxid/push-approvals-walkthrough.md",
  },
  {
    id: "9a0f6a4d-6a3a-4f8b-9b6c-001000000002",
    product: NYXID_PRODUCT,
    slug: "aes-envelope-encryption",
    title: "AES-256 envelope encryption, in plain English",
    description:
      "Why every secret has a wrapper, why the wrapper has its own wrapper, and how that lets us swap KMS providers without ever decrypting your tokens.",
    body: ENVELOPE_ENCRYPTION_BODY,
    tags: [TAGS.security, TAGS.encryption, TAGS.kms],
    series: SERIES_FOUNDATIONS,
    author: MARCUS,
    hero_image: {
      id: "file-envelope",
      filename_disk: "envelope.jpg",
      url: unsplash("photo-1465101046530-73398c7f28ca"),
      width: 1600,
      height: 1067,
      alt: "Moon over a bank of clouds at night.",
    },
    published_at: "2026-04-19T09:00:00Z",
    status: "published",
    content_commit_sha: "b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1",
    content_url:
      "https://github.com/ChronoAIProject/NyxID-content/blob/main/nyxid/aes-envelope-encryption.md",
  },
  {
    id: "9a0f6a4d-6a3a-4f8b-9b6c-001000000003",
    product: NYXID_PRODUCT,
    slug: "oauth-to-delegated-access",
    title: "From OAuth to delegated access tokens",
    description:
      "Token exchange (RFC 8693) is the thing nobody implements all the way. Here's how we wired it for service accounts and what we learned from the audit log.",
    body: OAUTH_DELEGATION_BODY,
    tags: [TAGS.oauth, TAGS.delegation, TAGS.security],
    series: null,
    author: CALVIN,
    hero_image: {
      id: "file-oauth",
      filename_disk: "oauth.jpg",
      url: unsplash("photo-1502134249126-9f3755a50d78"),
      width: 1600,
      height: 1067,
      alt: "Code displayed on a dark monitor.",
    },
    published_at: "2026-04-11T09:00:00Z",
    status: "published",
    content_commit_sha: "c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2",
    content_url:
      "https://github.com/ChronoAIProject/NyxID-content/blob/main/nyxid/oauth-to-delegated-access.md",
  },
  {
    id: "9a0f6a4d-6a3a-4f8b-9b6c-001000000004",
    product: NYXID_PRODUCT,
    slug: "designing-for-the-threshold",
    title: "Designing for the threshold: how we picked our purple",
    description:
      "Why we abandoned Tailwind's violet-500. What changed when we shifted six points warmer. And how we keep purple from becoming wallpaper.",
    body: DESIGN_BODY,
    tags: [TAGS.design, TAGS.color, TAGS.product],
    series: null,
    author: JAMIE,
    hero_image: {
      id: "file-design",
      filename_disk: "design.jpg",
      url: unsplash("photo-1419242902214-272b3f66ee7a"),
      width: 1600,
      height: 1067,
      alt: "The Milky Way over a dark mountain ridge.",
    },
    published_at: "2026-04-15T09:00:00Z",
    status: "published",
    content_commit_sha: "d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3",
    content_url:
      "https://github.com/ChronoAIProject/NyxID-content/blob/main/nyxid/designing-for-the-threshold.md",
  },
  // Draft — exists so the preview route has something to render.
  {
    id: "9a0f6a4d-6a3a-4f8b-9b6c-001000000005",
    product: NYXID_PRODUCT,
    slug: "audit-trail-postmortem",
    title: "What Nyx sees: an audit trail post-mortem",
    description:
      "A 4am page, a missing event, and the rabbit hole that followed. On why \"every action is logged\" is a promise that takes work to keep.",
    body: AUDIT_POSTMORTEM_BODY,
    tags: [TAGS.audit, TAGS.postmortem, TAGS.security],
    series: null,
    author: PRIYA,
    hero_image: {
      id: "file-audit",
      filename_disk: "audit.jpg",
      url: unsplash("photo-1551033406-611cf9a28f67"),
      width: 1600,
      height: 1067,
      alt: "Glowing keyboard in low light.",
    },
    published_at: null,
    status: "draft",
    content_commit_sha: "",
    content_url: "",
  },
];
