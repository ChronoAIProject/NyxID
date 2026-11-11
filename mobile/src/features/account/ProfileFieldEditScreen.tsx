import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { SheetScreenProps } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsPage,
  SettingsTextField,
  useSettingsHeaderAction,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { accountApi } from "../../lib/api/accountApi";
import { updateProfileSchema } from "../../lib/api/accountSchema";
import { resolveErrorMessage } from "../../lib/api/errorMessages";

export type ProfileField = "display_name" | "avatar_url";

const COPY: Record<
  ProfileField,
  { title: string; placeholder: string; footer: string }
> = {
  display_name: {
    title: "Display name",
    placeholder: "Your name",
    footer: "Shown to people you share agents and groups with.",
  },
  avatar_url: {
    title: "Avatar",
    placeholder: "https://example.com/avatar.png",
    footer:
      "An https link to a square image. Your initial is shown until you add one.",
  },
};

export function profileFieldTitle(field: ProfileField) {
  return COPY[field].title;
}

/** Settings › Account & security › one profile value, edited on its own page. */
export function ProfileFieldEditScreen({
  navigation,
  route,
}: SheetScreenProps<"ProfileEdit">) {
  const { t } = useAppLanguage();
  const field = route.params.field;
  const copy = COPY[field];
  const { isConnected } = useNetworkStatus();
  const client = useQueryClient();
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => accountApi.getProfile(signal),
  });
  const saved = profile.data?.[field] ?? "";
  const [value, setValue] = useState<string>();
  const draft = value ?? saved;
  const [error, setError] = useState<string>();
  const save = useMutation({
    mutationFn: accountApi.updateProfile,
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["account", "profile"] });
      navigation.goBack();
    },
    onError: (cause) => setError(resolveErrorMessage(cause)),
  });
  const changed = !!profile.data && draft.trim() !== saved;
  const submit = useCallback(() => {
    const parsed = updateProfileSchema.safeParse({ [field]: draft });
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message ?? "Check this value.");
      return;
    }
    setError(undefined);
    save.mutate(parsed.data);
  }, [draft, field, save]);
  useSettingsHeaderAction(navigation, {
    enabled: changed && isConnected && !save.isPending,
    busy: save.isPending,
    onPress: submit,
  });
  return (
    <SettingsPage>
      {!isConnected ? (
        <Notice message={t("You're offline. Connect to save changes.")} />
      ) : null}
      {error ? <Notice tone="error" message={error} /> : null}
      <SettingsGroup inset={false} footer={t(copy.footer)}>
        <SettingsTextField
          accessibilityLabel={t(copy.title)}
          value={draft}
          onChangeText={(text) => {
            setValue(text);
            setError(undefined);
          }}
          placeholder={
            field === "avatar_url" ? copy.placeholder : t(copy.placeholder)
          }
          autoFocus
          editable={!!profile.data && !save.isPending}
          returnKeyType="done"
          onSubmitEditing={() => {
            if (changed) submit();
          }}
          {...(field === "avatar_url"
            ? {
                keyboardType: "url" as const,
                autoCapitalize: "none" as const,
                autoCorrect: false,
              }
            : {
                autoCapitalize: "words" as const,
                textContentType: "name" as const,
              })}
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
