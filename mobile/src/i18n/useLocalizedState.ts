import { useCallback, useState } from 'react';
import { useAppLanguage } from './LanguageProvider';

/** A retained app message owns its pure formatter and captured values, rather
 * than a sentence formatted in yesterday's language. One formatter per state
 * slot; clearing/replacing/unmounting releases it. Never use for chat/user data.
 * Formatters must not perform effects or read mutable input fields. */
export function useLocalizedState<T = undefined>(): readonly [T | undefined, (value: (() => T | undefined) | undefined) => void];
export function useLocalizedState<T>(initial: T): readonly [T, (value: (() => T) | Extract<T, null | undefined>) => void];
export function useLocalizedState<T>(initial?: T) {
  useAppLanguage();
  const [format, setFormat] = useState<{ render: () => T | undefined }>(() => ({ render: () => initial }));
  const publish = useCallback((value: (() => T | undefined) | null | undefined) => {
    setFormat({ render: typeof value === 'function' ? value : () => value as T | undefined });
  }, []);
  return [format.render(), publish] as const;
}
