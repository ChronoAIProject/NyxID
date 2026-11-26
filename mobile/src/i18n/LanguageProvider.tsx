import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { deviceLanguages } from "./deviceLanguages";
import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  initialLanguage,
  LANGUAGE_STORAGE_KEY,
  type Language,
} from "./language";
import { translate, type TranslationValues } from "./translate";

import { setAppTranslationLanguage } from "./appTranslation";

type AppLanguage = {
  language: Language;
  choose: (language: Language) => void;
  persistenceError: boolean;
  retryPersistence: () => void;
  t: (phrase: string, values?: TranslationValues) => string;
};
const LanguageContext = createContext<AppLanguage>({
  language: "en",
  choose() {},
  persistenceError: false,
  retryPersistence() {},
  t: (phrase, values) => translate("en", phrase, values),
});

export function LanguageProvider({ children }: { children: ReactNode }) {
  const [language, setLanguage] = useState<Language>("en");
  const [loaded, setLoaded] = useState(false);
  const [persistenceError, setPersistenceError] = useState(false);
  const writes = useRef(Promise.resolve());
  const revision = useRef(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    let active = true;
    void AsyncStorage.getItem(LANGUAGE_STORAGE_KEY)
      .catch(() => null)
      .then((saved) => {
        if (!active) return;
        const initial = initialLanguage(
          saved,
          deviceLanguages({ includeLocaleFallback: true }),
        );
        setAppTranslationLanguage(initial);
        setLanguage(initial);
        setLoaded(true);
      });
    return () => {
      active = false;
      mounted.current = false;
    };
  }, []);
  const choose = useCallback((next: Language) => {
    setAppTranslationLanguage(next);
    setLanguage(next);
    const write = ++revision.current;
    setPersistenceError(false);
    writes.current = writes.current
      .catch(() => {})
      .then(() => AsyncStorage.setItem(LANGUAGE_STORAGE_KEY, next));
    void writes.current.then(
      () => {
        if (mounted.current && write === revision.current)
          setPersistenceError(false);
      },
      () => {
        if (mounted.current && write === revision.current)
          setPersistenceError(true);
      },
    );
  }, []);
  const value = useMemo<AppLanguage>(
    () => ({
      language,
      choose,
      persistenceError,
      retryPersistence: () => choose(language),
      t: (phrase, values) => translate(language, phrase, values),
    }),
    [language, choose, persistenceError],
  );
  if (!loaded) return null;
  return (
    <LanguageContext.Provider value={value}>
      {children}
    </LanguageContext.Provider>
  );
}

export function useAppLanguage(): AppLanguage {
  return useContext(LanguageContext);
}
