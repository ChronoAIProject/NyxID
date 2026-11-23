import AsyncStorage from "@react-native-async-storage/async-storage";
import { File, Paths } from "expo-file-system";
import { localizedError } from "../../i18n/localizedError";
import { assistantUploadsApi } from "../../lib/api/assistantUploadsApi";
import { getAuthAccountId, subscribeAuthAccount } from "../../lib/auth/authAccount";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
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
  const lifetime = new AbortController();
  const unsubscribe = subscribeAuthAccount(() => lifetime.abort());
  const assertOwner = uploadDraftAuthority(owner, epoch);
  const guard = () => {
    assertOwner();
    if (lifetime.signal.aborted) throw localizedError("Request cancelled");
  };
  const target = uploadDraftDestination(key);
  const expectedId = target?.name === "GroupChat"
    ? target.params.groupId
    : target?.params.conversationId;
  const kind = target?.name === "GroupChat" ? "groups" : "conversations";
  try {
    await uploadDraftStorage.discard(owner, key, guard, async ({ file, item }, conversationId) => {
      removeCachedFile(file.path);
      if (!item) return;
      const id = expectedId ?? conversationId;
      if (!id || (expectedId && conversationId && expectedId !== conversationId))
        throw localizedError("The upload draft is unavailable.");
      // The DELETE has its own deadline. Only the storage lane checkpoints it.
      await assistantUploadsApi.remove({ kind, id }, item.id, {
        expectedAccountId: owner,
        authorityGuard: guard,
        signal: lifetime.signal,
      });
    });
  } finally {
    unsubscribe();
  }
}
