import { ApiError } from "./ApiError";
import { createIdempotencyKey } from "./idempotency";
import {
  deleteCurrentUserAccountRequest,
  getCurrentUserProfileRequest,
  getChallengeRequest,
  getNotificationSettingsRequest,
  listApprovalsRequest,
  listApprovalRequestsRequest,
  listChallengesRequest,
  loginWithPasswordRequest,
  registerWithPasswordRequest,
  registerPushTokenRequest,
  revokeApprovalRequest,
  rotatePushTokenRequest,
  submitChallengeDecisionRequest,
  telegramDisconnectRequest,
  telegramLinkRequest,
  unregisterPushTokenRequest,
  updateNotificationSettingsRequest,
} from "./http";
import {
  AccountProfile,
  ApprovalItem,
  ChallengeDetail,
  DeleteAccountResponse,
  NotificationSettings,
  PageResponse,
  SubmitDecisionOptions,
  PushTokenRegisterRequest,
  PushTokenRegisterResponse,
  TelegramLinkInfo,
  UpdateNotificationSettingsInput,
} from "./types";

type LoginWithPasswordInput = {
  email: string;
  password: string;
  mfaCode?: string;
};

type LoginWithPasswordResponse = {
  userId: string;
  accessToken: string;
  expiresIn: number;
  refreshToken?: string;
};

type RegisterWithPasswordInput = {
  email: string;
  password: string;
  displayName?: string;
};

type RegisterWithPasswordResponse = {
  userId: string;
  message: string;
};

type SocialProvider = "google" | "github" | "apple";

function getApiBaseUrl(): string {
  const rawBaseUrl = process.env.EXPO_PUBLIC_API_BASE_URL ?? "http://localhost:3001/api/v1";
  const normalized = rawBaseUrl.replace(/\/+$/, "");

  if (normalized.endsWith("/mobile")) {
    return normalized.slice(0, -"/mobile".length);
  }

  return normalized;
}

export const mobileApi = {
  async loginWithPassword(input: LoginWithPasswordInput): Promise<LoginWithPasswordResponse> {
    const response = await loginWithPasswordRequest({
      email: input.email,
      password: input.password,
      mfa_code: input.mfaCode,
    });

    return {
      userId: response.user_id,
      accessToken: response.access_token,
      expiresIn: response.expires_in,
      refreshToken: response.refresh_token,
    };
  },
  async registerWithPassword(
    input: RegisterWithPasswordInput
  ): Promise<RegisterWithPasswordResponse> {
    const response = await registerWithPasswordRequest({
      email: input.email,
      password: input.password,
      display_name: input.displayName,
    });

    return {
      userId: response.user_id,
      message: response.message,
    };
  },
  getSocialAuthorizeUrl(provider: SocialProvider, redirectUri: string): string {
    const encodedRedirectUri = encodeURIComponent(redirectUri);
    return `${getApiBaseUrl()}/auth/social/${provider}?client=mobile&redirect_uri=${encodedRedirectUri}`;
  },
  async getChallenges(page = 1, perPage = 20): Promise<PageResponse<ChallengeDetail>> {
    return listChallengesRequest(page, perPage);
  },
  async getNotificationSettings(): Promise<NotificationSettings> {
    return getNotificationSettingsRequest();
  },
  async updateNotificationSettings(input: UpdateNotificationSettingsInput): Promise<NotificationSettings> {
    return updateNotificationSettingsRequest(input);
  },
  async telegramLink(): Promise<TelegramLinkInfo> {
    return telegramLinkRequest();
  },
  async telegramDisconnect(): Promise<{ message: string }> {
    return telegramDisconnectRequest();
  },
  async getChallengeById(challengeId: string): Promise<ChallengeDetail> {
    return getChallengeRequest(challengeId);
  },
  async submitDecision(
    challengeId: string,
    decision: "APPROVE" | "DENY",
    durationSec?: number,
    options?: SubmitDecisionOptions
  ): Promise<{ challenge_id: string; status: string; approval_id?: string }> {
    const idempotencyKey =
      options?.idempotencyKey ?? createIdempotencyKey("decision", challengeId);
    return submitChallengeDecisionRequest(challengeId, decision, durationSec, idempotencyKey);
  },
  async getApprovals(page = 1, perPage = 20): Promise<PageResponse<ApprovalItem>> {
    return listApprovalsRequest(page, perPage);
  },
  async getHistory(page = 1, perPage = 20): Promise<PageResponse<ChallengeDetail>> {
    // Preferred path: filter out PENDING server-side via the multi-
    // status list form. Client-side filtering would silently strand
    // decided items when an admin's org has enough pending items to
    // fill page 1 under include_admin_orgs — the first page comes
    // back empty of history and the screen renders the empty state
    // instead of paging further.
    //
    // Fallback: pre-376 backends only accept a single status value
    // and reject "approved,rejected,expired" with a 400. When the
    // mobile app ships before the backend is upgraded, the History
    // tab would otherwise break entirely. Fall back to fetching
    // without a status filter and dropping PENDING client-side.
    // That fallback is only safe because older backends also ignore
    // include_admin_orgs, so the response never contains admin-org
    // PENDING items that would fill a page — the original
    // personal-only behavior is preserved byte-for-byte.
    try {
      return await listApprovalRequestsRequest({
        page,
        per_page: perPage,
        status: "approved,rejected,expired",
      });
    } catch (error) {
      const isOldBackendValidationError =
        error instanceof ApiError &&
        error.statusCode === 400 &&
        error.errorKey === "validation_error";
      if (!isOldBackendValidationError) throw error;
      const response = await listApprovalRequestsRequest({ page, per_page: perPage });
      return {
        ...response,
        items: response.items.filter((item) => item.status !== "PENDING"),
      };
    }
  },
  async revoke(approvalId: string, orgId?: string | null): Promise<{ message: string }> {
    return revokeApprovalRequest(approvalId, orgId);
  },
  async registerPushToken(
    payload: PushTokenRegisterRequest
  ): Promise<PushTokenRegisterResponse> {
    return registerPushTokenRequest(payload);
  },
  async rotatePushToken(
    payload: PushTokenRegisterRequest
  ): Promise<PushTokenRegisterResponse> {
    return rotatePushTokenRequest(payload);
  },
  async unregisterPushToken(payload: PushTokenRegisterRequest): Promise<void> {
    await unregisterPushTokenRequest(payload);
  },
  async getAccountProfile(): Promise<AccountProfile> {
    return getCurrentUserProfileRequest();
  },
  async deleteAccount(): Promise<DeleteAccountResponse> {
    return deleteCurrentUserAccountRequest();
  },
};
