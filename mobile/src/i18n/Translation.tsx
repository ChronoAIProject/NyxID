import { Fragment, type ReactNode } from "react";
import { useAppLanguage } from "./LanguageProvider";
import { translationNodes } from "./translationNodes";

/** A complete locale-owned sentence whose named slots may retain links or emphasis. */
export function Translation({ message, values }: {
  message: string;
  values: Record<string, ReactNode>;
}) {
  const { t } = useAppLanguage();
  return <>{translationNodes(t(message), values).map((part, index) =>
    <Fragment key={index}>{part}</Fragment>,
  )}</>;
}
