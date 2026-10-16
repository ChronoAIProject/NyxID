import * as SecureStore from "expo-secure-store";
import { advanceAuthEpoch, getAuthEpoch } from "./authEpoch";
import { decodeJwtSub } from "./jwt";
import { createSessionPersistence } from "./sessionPersistence";

export type { StoredAuthSession } from "./sessionPersistence";

const persistence = createSessionPersistence({
  store: {
    getItem: SecureStore.getItemAsync,
    setItem: SecureStore.setItemAsync,
    deleteItem: SecureStore.deleteItemAsync,
  },
  getAuthEpoch,
  advanceAuthEpoch,
  decodeSubject: decodeJwtSub,
});

export const {
  loadStoredAuthSession,
  persistAuthSession,
  clearStoredAuthSession,
  persistRefreshedAuthSession,
} = persistence;
