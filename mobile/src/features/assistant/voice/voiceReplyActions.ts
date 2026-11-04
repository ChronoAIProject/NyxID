import { assistantLinkTarget } from "../assistantLinkTarget";

const URL_PATTERN = /https?:\/\/[^\s<>()[\]"'`]+/g;

/**
 * Whether a NyxBot reply carries something the person can act on in the app
 * (a hosted connect link, add-service, bot setup or account page), so the
 * voice chat can show that reply where it can be tapped during the call.
 */
export function hasAssistantActionLink(
  text: string,
  frontendUrls: string | readonly string[],
) {
  for (const match of text.matchAll(URL_PATTERN)) {
    const href = match[0].replace(/[.,;:!?]+$/, "");
    if (assistantLinkTarget(href, frontendUrls)) return true;
  }
  return false;
}

/** Clock skew allowance between the server's message time and this phone. */
export const VOICE_REPLY_SKEW_MS = 60_000;
