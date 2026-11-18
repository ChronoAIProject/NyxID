import { advanceAuthEpoch } from './authEpoch';
import { setAuthAccountId } from './authAccount';
import type { StoredAuthSession } from './sessionCoordinator';
import * as SecureStore from 'expo-secure-store';
import { analyticsConsentState } from '../analyticsConsentState';
import { createSessionCoordinator } from './sessionCoordinator';
export type { StoredAuthSession } from './sessionCoordinator';
const coordinator = createSessionCoordinator(SecureStore, analyticsConsentState.bindAccount);
export const loadStoredAuthSession = coordinator.load;
export function persistAuthSession(session: StoredAuthSession) {
  advanceAuthEpoch();
  return coordinator.persist(session);
}
export function clearStoredAuthSession() {
  setAuthAccountId(null);
  advanceAuthEpoch();
  return coordinator.clear();
}
export const commitRefreshedAuthSession = coordinator.commitRefresh;
export const authSessionGeneration = coordinator.generation;
