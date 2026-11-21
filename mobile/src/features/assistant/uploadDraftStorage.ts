import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../../lib/auth/authAccount";
import { createUploadDraftRecovery } from "./uploadDraftRecovery";

export const uploadDraftStorage = createUploadDraftRecovery(AsyncStorage);
let previous = getAuthAccountId();
subscribeAuthAccount(() => {
  const current = getAuthAccountId();
  if (previous && previous !== current)
    void uploadDraftStorage.clear(previous).catch(() => {});
  previous = current;
});
