import AsyncStorage from "@react-native-async-storage/async-storage";
import { File, Paths } from "expo-file-system";
import { localizedError } from "../../i18n/localizedError";
import { assistantUploadsApi } from "../../lib/api/assistantUploadsApi";
import { getAuthAccountId, subscribeAuthAccount } from "../../lib/auth/authAccount";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { withAttachmentUploadDeadline } from "./assistantUploads";
import { createUploadDraftRecovery, relativeCachePath, uploadCachePrefix, uploadDraftDestination } from "./uploadDraftRecovery";

export const uploadDraftStorage = createUploadDraftRecovery(AsyncStorage);

function removeCachedFile(path?: string) {
  const prefix = uploadCachePrefix(Paths.cache.uri);
  if (!path || relativeCachePath(prefix + path, prefix) !== path) return;
  const cached = new File(prefix + path);
  if (cached.exists) cached.delete();
}

let previousAccount = getAuthAccountId();
subscribeAuthAccount(() => {
  const current = getAuthAccountId();
  if (previousAccount && previousAccount !== current) {
    // Sign-out clears composer recovery as before. An explicit discard already
    // in progress retains only its blocked cleanup record for its original owner.
    void uploadDraftStorage.clear(previousAccount, (draft) => {
      for (const { file } of draft.rows) removeCachedFile(file.path);
    }).catch(() => {});
  }
  previousAccount = current;
});

export function uploadDraftAuthority(owner: string, epoch: number) {
  return () => {
    if (getAuthAccountId() !== owner || getAuthEpoch() !== epoch)
      throw localizedError("This composer is no longer available.");
  };
}

export async function discardStoredUploadDraft(owner: string, key: string, epoch: number) {
  const authorityGuard = uploadDraftAuthority(owner, epoch);
  const lifetime = new AbortController();
  const unsubscribe = subscribeAuthAccount(() => lifetime.abort());
  try {
    await uploadDraftStorage.discard(owner, key, (draft) =>
      withAttachmentUploadDeadline(async (signal) => {
        authorityGuard();
        const target = uploadDraftDestination(key);
        const expectedId = target?.name === "GroupChat"
          ? target.params.groupId
          : target?.params.conversationId;
        const id = expectedId ?? draft.conversationId;
        const kind = target?.name === "GroupChat" ? "groups" : "conversations";
        for (const { file, item } of draft.rows) {
          authorityGuard();
          if (signal.aborted) throw localizedError("Request cancelled");
          removeCachedFile(file.path);
          if (item) {
            if (!id || expectedId && draft.conversationId && draft.conversationId !== id)
              throw localizedError("The upload draft is unavailable.");
            await assistantUploadsApi.remove({ kind, id }, item.id, {
              expectedAccountId: owner,
              authorityGuard,
              signal,
            });
          }
        }
        authorityGuard();
      }, lifetime.signal, 15_000,
      () => localizedError("The draft could not be discarded. Try again.")),
    );
  } finally {
    unsubscribe();
  }
}
