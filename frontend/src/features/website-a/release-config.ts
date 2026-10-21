import { solutionSlugs } from "./solution-inventory";

// Literal build-time switches let Vite remove disabled component branches.
export const NEWSLETTER_ENABLED =
  import.meta.env.VITE_WEBSITE_NEWSLETTER_ENABLED === "true";
export const CONTACT_CHAT_ENABLED =
  import.meta.env.VITE_WEBSITE_CONTACT_CHAT_ENABLED === "true";
export const STATUS_ENABLED =
  import.meta.env.VITE_WEBSITE_STATUS_ENABLED === "true";
export const BLOG_MENU_ENABLED =
  import.meta.env.VITE_WEBSITE_BLOG_MENU_ENABLED === "true";
export type ReleaseEnvironment = Record<string, unknown>;
export interface NewsletterConfig {
  endpoint: string;
  privacyUrl: string;
  unsubscribeUrl: string;
}
export interface ChatConfig {
  endpoint: string;
}
export interface WebsiteRelease {
  blogMenu: boolean;
  newsletter: NewsletterConfig | null;
  contactChat: ChatConfig | null;
  footerStatus: string | null;
  solutions: Partial<Record<(typeof solutionSlugs)[number], string>>;
}
function publicUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
      ? url.href
      : null;
  } catch {
    return null;
  }
}
function endpoint(value: unknown): string | null {
  return typeof value === "string" &&
    /^\/api\/[a-zA-Z0-9/_-]+$/.test(value) &&
    !value.includes("//")
    ? value
    : null;
}
function accepted(value: unknown) {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    value.length <= 200 &&
    !/[\r\n]/.test(value)
  );
}
function requireConfig(condition: unknown, feature: string): asserts condition {
  if (!condition)
    throw new Error(
      `Invalid website release configuration: ${feature}. Keep the switch off until adapter configuration and acceptance are complete.`,
    );
}
/** Public build configuration only. No credentials or backend autodetection. */
export function resolveWebsiteRelease(env: ReleaseEnvironment): WebsiteRelease {
  const release: WebsiteRelease = {
    blogMenu: env.VITE_WEBSITE_BLOG_MENU_ENABLED === "true",
    newsletter: null,
    contactChat: null,
    footerStatus: null,
    solutions: {},
  };
  if (env.VITE_WEBSITE_NEWSLETTER_ENABLED === "true") {
    const api = endpoint(env.VITE_WEBSITE_NEWSLETTER_ENDPOINT),
      privacyUrl = publicUrl(env.VITE_WEBSITE_NEWSLETTER_PRIVACY_URL),
      unsubscribeUrl = publicUrl(env.VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL);
    requireConfig(
      api &&
        privacyUrl &&
        unsubscribeUrl &&
        accepted(env.VITE_WEBSITE_NEWSLETTER_ACCEPTANCE),
      "newsletter",
    );
    release.newsletter = { endpoint: api, privacyUrl, unsubscribeUrl };
  }
  if (env.VITE_WEBSITE_CONTACT_CHAT_ENABLED === "true") {
    const api = endpoint(env.VITE_WEBSITE_CONTACT_CHAT_ENDPOINT);
    requireConfig(
      api && accepted(env.VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE),
      "contact chat",
    );
    release.contactChat = { endpoint: api };
  }
  if (env.VITE_WEBSITE_STATUS_ENABLED === "true") {
    const url = publicUrl(env.VITE_WEBSITE_STATUS_URL);
    requireConfig(
      url &&
        !(
          new URL(url).origin === "https://nyx.chrono-ai.fun" &&
          new URL(url).pathname === "/"
        ) &&
        !/^\/(?:api|health)(?:\/|$)/.test(new URL(url).pathname) &&
        accepted(env.VITE_WEBSITE_STATUS_ACCEPTANCE),
      "status",
    );
    release.footerStatus = url;
  }
  const raw = env.VITE_WEBSITE_SOLUTION_ACTIVATIONS;
  if (typeof raw === "string" && raw.trim()) {
    let entries: unknown;
    try {
      entries = JSON.parse(raw);
    } catch {
      requireConfig(false, "solution JSON");
    }
    requireConfig(
      entries && typeof entries === "object" && !Array.isArray(entries),
      "solutions",
    );
    for (const [slug, value] of Object.entries(entries)) {
      requireConfig(
        (solutionSlugs as readonly string[]).includes(slug) &&
          value &&
          typeof value === "object",
        "solution slug",
      );
      const { target, acceptance } = value as Record<string, unknown>;
      // Activation targets existing authenticated workspaces; auth keeps its own
      // trusted return_to validation. Do not introduce a new auth protocol here.
      requireConfig(
        typeof target === "string" &&
          /^\/(?:assistant|dashboard)(?:\?|$)/.test(target) &&
          !/[\r\n\\#]/.test(target) &&
          accepted(acceptance),
        "solution target",
      );
      const url = new URL(target, "https://nyx.chrono-ai.fun");
      requireConfig(
        !["return_to", "redirect_uri", "code", "token"].some((key) =>
          url.searchParams.has(key),
        ),
        "solution target query",
      );
      release.solutions[slug as (typeof solutionSlugs)[number]] =
        url.pathname +
        (url.searchParams.size ? `?${url.searchParams.toString()}` : "");
    }
  }
  return release;
}
export const websiteRelease = resolveWebsiteRelease({
  VITE_WEBSITE_BLOG_MENU_ENABLED: import.meta.env
    .VITE_WEBSITE_BLOG_MENU_ENABLED,
  VITE_WEBSITE_NEWSLETTER_ENABLED: import.meta.env
    .VITE_WEBSITE_NEWSLETTER_ENABLED,
  VITE_WEBSITE_NEWSLETTER_ENDPOINT: import.meta.env
    .VITE_WEBSITE_NEWSLETTER_ENDPOINT,
  VITE_WEBSITE_NEWSLETTER_PRIVACY_URL: import.meta.env
    .VITE_WEBSITE_NEWSLETTER_PRIVACY_URL,
  VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL: import.meta.env
    .VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL,
  VITE_WEBSITE_NEWSLETTER_ACCEPTANCE: import.meta.env
    .VITE_WEBSITE_NEWSLETTER_ACCEPTANCE,
  VITE_WEBSITE_CONTACT_CHAT_ENABLED: import.meta.env
    .VITE_WEBSITE_CONTACT_CHAT_ENABLED,
  VITE_WEBSITE_CONTACT_CHAT_ENDPOINT: import.meta.env
    .VITE_WEBSITE_CONTACT_CHAT_ENDPOINT,
  VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE: import.meta.env
    .VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE,
  VITE_WEBSITE_STATUS_ENABLED: import.meta.env.VITE_WEBSITE_STATUS_ENABLED,
  VITE_WEBSITE_STATUS_URL: import.meta.env.VITE_WEBSITE_STATUS_URL,
  VITE_WEBSITE_STATUS_ACCEPTANCE: import.meta.env
    .VITE_WEBSITE_STATUS_ACCEPTANCE,
  VITE_WEBSITE_SOLUTION_ACTIVATIONS: import.meta.env
    .VITE_WEBSITE_SOLUTION_ACTIVATIONS,
});
export const websiteVisibility = {
  blogMenu: websiteRelease.blogMenu,
  newsletter: Boolean(websiteRelease.newsletter),
  contactChat: Boolean(websiteRelease.contactChat),
  footerStatus: Boolean(websiteRelease.footerStatus),
  solutions: Object.keys(websiteRelease.solutions).sort(),
};

// Effective public settings only; acceptance references never enter this snapshot.
export function canonicalWebsiteConfiguration(release: WebsiteRelease) {
  return JSON.stringify({
    ...release,
    solutions: Object.fromEntries(
      Object.entries(release.solutions).sort(([a], [b]) => a.localeCompare(b)),
    ),
  });
}
export const websiteConfiguration =
  canonicalWebsiteConfiguration(websiteRelease);
