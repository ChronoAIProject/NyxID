import { translateApp } from "../../i18n/appTranslation";
/** Presentation shared by password and social MFA prompts. */
export function mfaPrompt(provider: string | null, recovery: boolean) {
  const name = provider === 'github' ? 'GitHub' : provider === 'google' ? 'Google' : provider === 'apple' ? 'Apple' : 'social';
  return {
    showPasswordFields: provider === null,
    title: provider ? translateApp("Verify your {{name}} sign-in", { name: name})
      : translateApp('Verify your sign-in'),
    label: recovery ? translateApp('Recovery code')
      : translateApp('Authenticator code'),
    placeholder: recovery ? translateApp('Paste a recovery code') : '000000',
    toggle: recovery ? translateApp('Use an authenticator code')
      : translateApp('Use a recovery code'),
  };
}
