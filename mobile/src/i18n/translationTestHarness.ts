import { translationNodes } from "./translationNodes";
import { translate } from "./translate";
import * as translation from "./appTranslation";
export const englishTranslation = (phrase: string, values?: Record<string, string | number>) => translate("en", phrase, values);
export const richTranslationMock = {
  Translation: ({ message, values }: { message: string; values: Record<string, unknown> }) =>
    ({ type: "Fragment", props: { children: translationNodes(message, values) } }),
};
export const translationTestMocks = Object.fromEntries(
  ["../", "../../", "../../../", "../../../../"].flatMap((prefix) => [
    [
      prefix + "i18n/LanguageProvider",
      {
        useAppLanguage: () => ({
          language: "en",
          t: englishTranslation,
        }),
      },
    ],
    [prefix + "i18n/appTranslation", translation],
    [prefix + "i18n/Translation", richTranslationMock],
  ]),
);
