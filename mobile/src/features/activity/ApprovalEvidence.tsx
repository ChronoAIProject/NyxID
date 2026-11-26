import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import type { ChallengeDetail } from "../../lib/api/types";

/** The request's expiry as the reviewer's local time, never guessed. */
export function approvalExpiryLabel(challenge: ChallengeDetail) {
  const expiry = Date.parse(challenge.expires_at);
  return Number.isFinite(expiry)
    ? new Date(expiry).toLocaleString(appLocale())
    : translateApp("Not verified");
}

/** Tool arguments pretty-printed when they are JSON, verbatim otherwise. */
export function approvalArgumentsText(challenge: ChallengeDetail) {
  const text = challenge.tool_arguments;
  if (!text) return undefined;
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

/** The exact tool arguments under review, selectable for copying. */
export function ApprovalEvidence({
  challenge,
}: {
  challenge: ChallengeDetail;
}) {
  useAppLanguage();
  const argumentsText = approvalArgumentsText(challenge);
  if (!argumentsText) return null;
  return (
    <SettingsGroup title={translateApp("Tool arguments")} inset={false}>
      <SettingsRow label={argumentsText} mono selectable />
    </SettingsGroup>
  );
}
