/** Presentation shared by password and social MFA prompts. */
export function mfaPrompt(provider: string | null, recovery: boolean) {
  const name = provider === 'github' ? 'GitHub' : provider === 'google' ? 'Google' : provider === 'apple' ? 'Apple' : 'social';
  return {
    showPasswordFields: provider === null,
    title: provider ? `Verify your ${name} sign-in` : 'Verify your sign-in',
    label: recovery ? 'Recovery code' : 'Authenticator code',
    placeholder: recovery ? 'Paste a recovery code' : '000000',
    toggle: recovery ? 'Use an authenticator code' : 'Use a recovery code',
  };
}
