import { findSkillTemplate, type SkillTemplate } from "./skill-templates";

const STORAGE_KEY = "nyxid:skill-template-setup";
const OPEN_DRAFT_KEY = "nyxid:skill-template-draft";
const COOKIE_PREFIX = "nyxid_skill_setup_";
const LIFETIME_SECONDS = 30 * 60;

export interface SkillSetupSearch {
  readonly skill: SkillTemplate["slug"];
  readonly skillSetup: string;
}

interface PendingSetup extends SkillSetupSearch {
  readonly expiresAt: number;
  readonly owner?: string;
  readonly ssoStarted?: boolean;
}

interface OpenTemplateDraft extends SkillSetupSearch {
  readonly owner: string;
  readonly agentId: string;
}

export function isSkillSetupId(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{32}$/.test(value);
}

function removeTabSetup() {
  sessionStorage.removeItem(STORAGE_KEY);
}

function cookieName(id: string) {
  return `${COOKIE_PREFIX}${id}`;
}

function setCookie(id: string, value: string) {
  const maxAge = value ? LIFETIME_SECONDS : 0;
  const secure = location.protocol === "https:" ? "; Secure" : "";
  document.cookie = `${cookieName(id)}=${value}; Path=/; SameSite=Lax; Max-Age=${maxAge}${secure}`;
}

function readPending(): PendingSetup | undefined {
  try {
    const entry = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "null") as Partial<PendingSetup> | null;
    if (!entry || !isSkillSetupId(entry.skillSetup)) return;
    const skill = findSkillTemplate(entry.skill)?.slug;
    const cookie = `${cookieName(entry.skillSetup)}=${skill}`;
    if (!skill || typeof entry.expiresAt !== "number" || entry.expiresAt <= Date.now()) {
      clearPendingSkillSetup();
      return;
    }
    if (!document.cookie.split(/;\s*/).includes(cookie)) {
      removeTabSetup();
      return;
    }
    return { ...entry, skill, skillSetup: entry.skillSetup, expiresAt: entry.expiresAt };
  } catch {
    return;
  }
}

export function clearPendingSkillSetup() {
  let entry: Partial<PendingSetup> | null = null;
  try {
    entry = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "null") as Partial<PendingSetup> | null;
  } catch {
    // Remove malformed state if access to storage is still available.
  }
  try {
    removeTabSetup();
    if (isSkillSetupId(entry?.skillSetup)) setCookie(entry.skillSetup, "");
  } catch {
    // Browsers can disable one or both forms of storage.
  }
}

export function createSkillSetup(skill: SkillTemplate["slug"], owner?: string): SkillSetupSearch {
  clearPendingSkillSetup();
  const skillSetup = Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, "0")).join("");
  try {
    const setup: PendingSetup = { skill, skillSetup, owner, expiresAt: Date.now() + LIFETIME_SECONDS * 1000 };
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(setup));
    setCookie(skillSetup, skill);
  } catch {
    // The verified chat preview still opens. The owner can attach from there.
  }
  return { skill, skillSetup };
}

/** The nested auth return is the fixed assistant route with exactly two params. */
export function skillSetupReturn(value: string | undefined): SkillSetupSearch | undefined {
  if (!value) return;
  try {
    const target = new URL(value, location.origin);
    const skill = findSkillTemplate(target.searchParams.get("skill"))?.slug;
    const skillSetup = target.searchParams.get("skillSetup");
    if (target.origin === location.origin && target.pathname === "/assistant" && !target.hash &&
      Array.from(target.searchParams).length === 2 && skill && isSkillSetupId(skillSetup)) {
      return { skill, skillSetup };
    }
  } catch {
    return;
  }
}

function matchesSetup(pending: PendingSetup, setup: SkillSetupSearch | undefined) {
  return pending.skill === setup?.skill && pending.skillSetup === setup.skillSetup;
}

/** Mark the return fallback only when the browser actually goes to an SSO provider. */
export function beginSkillSetupSso(returnTo: string | undefined) {
  const pending = readPending();
  if (!pending) return;
  if (!matchesSetup(pending, skillSetupReturn(returnTo))) {
    clearPendingSkillSetup();
    return;
  }
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify({ ...pending, ssoStarted: true }));
  } catch {
    // The exact return URL remains usable.
  }
}

/** Returning to the auth form ends this provider attempt. The explicit return can still be retried. */
export function endSkillSetupSsoOnAuthBack() {
  if (location.pathname !== "/login" && location.pathname !== "/register") return;
  const pending = readPending();
  const returnTo = new URLSearchParams(location.search).get("return_to") ?? undefined;
  if (!pending?.ssoStarted || !matchesSetup(pending, skillSetupReturn(returnTo))) return;
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify({ ...pending, ssoStarted: false }));
  } catch {
    clearPendingSkillSetup();
  }
}

/** Use the tab's pending selection only on a fresh, ordinary SSO landing. */
export function pendingSkillSetupLanding(): SkillSetupSearch | undefined {
  const pending = readPending();
  const [navigation] = performance.getEntriesByType("navigation") as PerformanceNavigationTiming[];
  if (pending?.ssoStarted && !pending.owner && navigation?.type === "navigate" &&
    (!document.referrer || new URL(document.referrer).origin !== location.origin || navigation.redirectCount > 0)) {
    return { skill: pending.skill, skillSetup: pending.skillSetup };
  }
}

/** A different authenticated destination takes precedence over an old setup. */
export function discardUnrelatedSkillSetup(path: string, search: string) {
  const pending = readPending();
  if (!pending) return;
  const params = new URLSearchParams(search);
  const setup = path === "/assistant" ? skillSetupReturn(`${path}${search}`) :
    path === "/login" || path === "/register" ? skillSetupReturn(params.get("return_to") ?? undefined) : undefined;
  if (!matchesSetup(pending, setup)) clearPendingSkillSetup();
}

/** Consume the browser intent before allowing an automatic attach. */
export function claimSkillSetup(setup: SkillSetupSearch, owner: string): boolean {
  const pending = readPending();
  if (!pending || !matchesSetup(pending, setup)) return false;
  clearPendingSkillSetup();
  return !pending.owner || pending.owner === owner;
}

export function readOpenTemplateDraft(setup: SkillSetupSearch, owner: string): string | undefined {
  try {
    const entry = JSON.parse(sessionStorage.getItem(OPEN_DRAFT_KEY) ?? "null") as Partial<OpenTemplateDraft> | null;
    if (entry?.skill !== setup.skill || entry.skillSetup !== setup.skillSetup || entry.owner !== owner) return;
    const agentId = entry.agentId;
    if (typeof agentId === "string" && /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(agentId)) return agentId;
  } catch {
    // A blocked storage read can still use the ordinary preview.
  }
}

export function saveOpenTemplateDraft(setup: SkillSetupSearch, owner: string, agentId: string) {
  const entry: OpenTemplateDraft = { ...setup, owner, agentId };
  try {
    sessionStorage.setItem(OPEN_DRAFT_KEY, JSON.stringify(entry));
  } catch {
    // The current chat can still be used when browser storage is blocked.
  }
}

export function clearOpenTemplateDraft(setupId: string, owner: string) {
  try {
    const entry = JSON.parse(sessionStorage.getItem(OPEN_DRAFT_KEY) ?? "null") as Partial<OpenTemplateDraft> | null;
    if (entry?.skillSetup === setupId && entry.owner === owner) sessionStorage.removeItem(OPEN_DRAFT_KEY);
  } catch {
    // Draft cleanup is best-effort when storage is blocked.
  }
}
