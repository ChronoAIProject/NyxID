import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState } from "react";
import { Modal, Pressable, ScrollView, View } from "react-native";
import { Check, X } from "lucide-react-native";
import { Field, Label } from "../../components/workspace/WorkspaceUI";
import {
  workspaceCloseControlStyle,
  WORKSPACE_CLOSE_ICON_SIZE,
} from "../../components/workspace/workspaceCloseControl";
import { useTheme } from "../../theme/ThemeContext";
import { connectionOwner } from "../services/serviceContracts";
import { composerConnectionGroups } from "./composerConnection";
import type { ComposerConnectionState } from "./useComposerConnection";

export function ComposerConnectionPicker({
  state,
}: {
  state: ComposerConnectionState;
}) {
  const { colors } = useTheme();
  const [search, setSearch] = useState("");
  useEffect(() => {
    if (!state.open) setSearch("");
  }, [state.open]);
  const groups = composerConnectionGroups(state.connections, search);
  const connection = state.connection;
  return (
    <>
      {connection ? (
        <View
          accessibilityLabel="Connection for this message"
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: 8,
            marginHorizontal: 8,
            paddingLeft: 8,
            borderRadius: 8,
            borderWidth: 1,
            borderColor: colors.border,
          }}
        >
          <ServiceIcon
            slug={connection.catalog_service_slug ?? connection.slug}
            iconUrl={connection.icon_url || connection.catalog_icon_url}
            size="2xs"
          />
          <Label
            style={{
              flex: 1,
              fontSize: 12,
              color: state.unavailable ? colors.danger : colors.textSecondary,
            }}
          >
            {connection.catalog_service_name ?? connection.label} ·{" "}
            {connection.label} · {connectionOwner(connection)}
            {state.unavailable
              ? " · Unavailable — choose another connection or remove"
              : " · This message"}
          </Label>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Remove connection for this message"
            disabled={state.locked}
            onPress={state.remove}
            style={{
              minWidth: 44,
              minHeight: 44,
              alignItems: "center",
              justifyContent: "center",
              opacity: state.locked ? 0.5 : 1,
            }}
          >
            <X size={14} color={colors.textSecondary} />
          </Pressable>
        </View>
      ) : null}
      <Modal
        visible={state.open}
        transparent
        animationType="fade"
        onRequestClose={state.close}
      >
        <View
          style={{
            flex: 1,
            justifyContent: "center",
            padding: 20,
            backgroundColor: colors.sheetScrim,
          }}
          accessibilityViewIsModal
        >
          <View
            style={{
              backgroundColor: colors.card,
              borderColor: colors.border,
              borderWidth: 1,
              borderRadius: 16,
              maxHeight: "85%",
              width: "100%",
              maxWidth: 520,
              alignSelf: "center",
              overflow: "hidden",
            }}
          >
            <View
              style={{
                flexDirection: "row",
                alignItems: "center",
                gap: 8,
                padding: 12,
              }}
            >
              <Label
                accessibilityRole="header"
                numberOfLines={1}
                maxFontSizeMultiplier={1.25}
                style={{ fontSize: 18, flex: 1 }}
              >
                Connection for this message
              </Label>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Close connections"
                onPress={state.close}
                style={workspaceCloseControlStyle(colors.cardSoft)}
              >
                <X
                  size={WORKSPACE_CLOSE_ICON_SIZE}
                  color={colors.textSecondary}
                />
              </Pressable>
            </View>
            <ScrollView
              keyboardShouldPersistTaps="handled"
              keyboardDismissMode="interactive"
              contentContainerStyle={{ padding: 16, gap: 8 }}
            >
              <Label muted>
                Choose the account to use for this message. Manage your saved
                connection order in Agent Details.
              </Label>
              <Field
                label="Search connections"
                placeholder="Search services or accounts…"
                value={search}
                onChangeText={setSearch}
              />
              {state.loading ? (
                <Label accessibilityLiveRegion="polite">
                  Loading connections…
                </Label>
              ) : null}
              {state.error ? (
                <>
                  <Label accessibilityRole="alert">
                    Could not load connections.
                  </Label>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Retry"
                    disabled={state.locked}
                    onPress={state.retry}
                    style={{ minHeight: 44, justifyContent: "center" }}
                  >
                    <Label>Retry</Label>
                  </Pressable>
                </>
              ) : (
                groups.map((group) => (
                  <View key={group.id} style={{ gap: 4 }}>
                    <Label accessibilityRole="header" style={{ fontSize: 12 }}>
                      {group.name}
                    </Label>
                    {group.connections.map((key) => (
                      <Pressable
                        key={key.id}
                        accessibilityRole="button"
                        accessibilityLabel={`${key.label}, ${connectionOwner(key)}, ${key.slug}`}
                        accessibilityState={{
                          selected: connection?.id === key.id,
                          disabled: state.locked,
                        }}
                        disabled={state.locked}
                        onPress={() => state.select(key.id)}
                        style={{
                          minHeight: 44,
                          flexDirection: "row",
                          alignItems: "center",
                          gap: 8,
                          padding: 8,
                          borderRadius: 8,
                          backgroundColor:
                            connection?.id === key.id
                              ? colors.cardSoft
                              : "transparent",
                        }}
                      >
                        <ServiceIcon
                          slug={key.catalog_service_slug ?? key.slug}
                          iconUrl={key.icon_url || key.catalog_icon_url}
                          size="sm"
                        />
                        <View style={{ flex: 1 }}>
                          <Label style={{ fontSize: 14 }}>{key.label}</Label>
                          <Label muted style={{ fontSize: 11 }}>
                            {connectionOwner(key)} · {key.slug}
                          </Label>
                        </View>
                        {connection?.id === key.id ? (
                          <Check size={16} color={colors.textSecondary} />
                        ) : null}
                      </Pressable>
                    ))}
                  </View>
                ))
              )}
              {!state.loading && !state.error && !groups.length ? (
                <Label muted>
                  {search.trim()
                    ? "No matching connections."
                    : "No enabled connections are available."}
                </Label>
              ) : null}
              <Label muted style={{ fontSize: 11 }}>
                Organization connections still require approval in Auto and Ask
                every time.
              </Label>
            </ScrollView>
          </View>
        </View>
      </Modal>
    </>
  );
}
