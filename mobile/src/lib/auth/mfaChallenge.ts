import { ApiError } from '../api/ApiError';
/** Challenge tokens stay in memory and out of enumerable error properties. */
export class MfaChallengeError extends ApiError {
  #token: string;
  constructor(token: string) {
    super({ errorKey: 'mfa_required', errorCode: 2002, statusCode: 403, message: 'Enter an authenticator or recovery code.' });
    this.#token = token;
  }
  async verify<T>(code: string, request: (body: { code: string; mfa_token: string }) => Promise<T>): Promise<T> {
    if (!/^(?:\d{6}|[A-Za-z0-9_-]{8}|[a-fA-F0-9]{32})$/.test(code)) throw new Error('Enter an authenticator or recovery code.');
    return request({ code, mfa_token: this.#token });
  }
}

export function isMfaChallengeToken(value: unknown): value is string {
  return typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}

/** Accept only the documented first-factor response, without retaining its body. */
export function mfaChallengeFromResponse(payload: unknown, status: number): MfaChallengeError | null {
  if ((status !== 403 && status !== 400) || !payload || typeof payload !== 'object') return null;
  const row = payload as Record<string, unknown>;
  if (status === 400 && row.error !== 'mfa_required') return null;
  if (row.error_code !== 2002 || !isMfaChallengeToken(row.session_token)) return null;
  return new MfaChallengeError(row.session_token);
}
