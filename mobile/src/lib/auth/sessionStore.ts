import { advanceAuthEpoch } from './authEpoch';
import { setAuthAccountId } from './authAccount';
import type { SessionStorage, StoredAuthSession } from './sessionCoordinator';
import * as SecureStore from 'expo-secure-store';
import { analyticsConsentState } from '../analyticsConsentState';
import { createSessionCoordinator } from './sessionCoordinator';
export type { StoredAuthSession } from './sessionCoordinator';
/** iOS can launch the app in the background while the device is locked
 * (silent pushes, relaunch after the system reclaimed it). The default
 * WHEN_UNLOCKED class makes every read throw there, which surfaced as a
 * logout. Session secrets stay device-bound and out of encrypted backups. */
export const SESSION_KEYCHAIN_OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
};
const sessionStorage: SessionStorage = {
  getItemAsync: (key) =>
    SecureStore.getItemAsync(key, SESSION_KEYCHAIN_OPTIONS),
  setItemAsync: (key, value) =>
    SecureStore.setItemAsync(key, value, SESSION_KEYCHAIN_OPTIONS),
  deleteItemAsync: (key) =>
    SecureStore.deleteItemAsync(key, SESSION_KEYCHAIN_OPTIONS),
};
const coordinator = createSessionCoordinator(
  sessionStorage, analyticsConsentState.bindAccount);
export const loadStoredAuthSession = coordinator.load;
export const peekStoredAuthSession = coordinator.peek;
export function persistAuthSession(session: StoredAuthSession) {
  advanceAuthEpoch();
  return coordinator.persist(session);
}
export async function clearStoredAuthSession() {
  const clearing = coordinator.clear();
  const expected = coordinator.generation();
  await clearing;
  if (coordinator.generation() === expected) {
    setAuthAccountId(null);
    advanceAuthEpoch();
  }
}
export const commitRefreshedAuthSession = coordinator.commitRefresh;
export const authSessionGeneration = coordinator.generation;
