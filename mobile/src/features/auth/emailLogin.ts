import { translateApp } from "../../i18n/appTranslation";
import { isApiError } from "../../lib/api/ApiError";

export function requiresTotp(error: unknown): boolean {
  return (
    isApiError(error) &&
    (error.errorCode === 2002 || error.errorKey === "mfa_required")
  );
}

export function passwordLoginInput(
  email: string,
  password: string,
  mfaRequired: boolean,
  code: string,
) {
  if (!email.trim() || !password)
    throw new Error(translateApp("Enter your email and password."));
  const mfaCode = code.trim();
  if (mfaRequired && !/^\d{6}$/.test(mfaCode))
    throw new Error(
      translateApp("Enter the six-digit code from your authenticator."),
    );
  return { email: email.trim(), password, ...(mfaRequired ? { mfaCode } : {}) };
}
