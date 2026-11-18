import type { ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { ChevronDown, ChevronUp, Minus, Plus } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import type { QuickAccessItem } from "../../lib/api/quickAccessSchema";
import { chatAgentLabel } from "../assistant/assistantAgentContext";
import {
  addQuickAccess,
  moveQuickAccess,
  quickAccessCandidates,
  quickAccessDestinationLabel,
  quickAccessFull,
  removeQuickAccess,
  resolveQuickAccess,
  type QuickAccessEntry,
} from "./quickAccessModel";
import { QuickAccessGlyph } from "./QuickAccessGlyph";
import { useQuickAccess } from "./useQuickAccess";
import { useQuickAccessSources } from "./useQuickAccessSources";

/**
 * Settings › Quick access (the Duo, where the rail exists): which entries the
 * rail's shortcuts capsule holds, in order: side-menu destinations and pinned
 * agents or crews. Saved to the account (optimistic; a failed save puts the
 * previous list back). Up to 12; reset returns to the default list.
 */
export function QuickAccessSettingsScreen(
  _props: SheetScreenProps<"QuickAccessSettings">,
) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const quickAccess = useQuickAccess();
  const { items, customized } = quickAccess.state;
  const sources = useQuickAccessSources({ agents: true, crews: true });
  // Editing waits for the saved list: a change made over the placeholder
  // default would overwrite it.
  const editable = quickAccess.loaded;
  const full = quickAccessFull(items);
  const change = (next: QuickAccessItem[]) => {
    quickAccess.clearSaveError();
    quickAccess.save(next);
  };
  const candidates = quickAccessCandidates(items, sources);
  const glyph = (entry: QuickAccessEntry) => (
    <QuickAccessGlyph
      entry={entry}
      color={colors.textPrimary}
      surface={colors.card}
      size={17}
    />
  );
  // Each saved item: the rail's entry, or a placeholder for an agent or crew
  // that was deleted (or is no longer shared) so it can be removed here.
  const rows = items.map((item, index) => ({
    item,
    index,
    entry: resolveQuickAccess([item], { ...sources, notificationsEnabled: true })[0],
  }));
  const loadingSources = (item: QuickAccessItem) =>
    (item.kind === "agent" && !sources.agents) ||
    (item.kind === "crew" && !sources.crews);
  return (
    <SettingsPage>
      {!quickAccess.loaded && quickAccess.loadError ? (
        <SettingsGroup footer={t("Couldn't load quick access.")}>
          <SettingsRow
            label={t("Retry")}
            tone="action"
            onPress={quickAccess.retry}
          />
        </SettingsGroup>
      ) : null}
      <SettingsGroup
        title={t("Your shortcuts")}
        footer={
          quickAccess.saveError
            ? t("Couldn't save quick access. Your previous list is back.")
            : t(
                "Shown in the column under the clock, in this order. Saved to your account, so your other devices show the same list.",
              )
        }
      >
        {!editable ? (
          <SettingsRow label={t("Loading…")} disabled />
        ) : rows.length === 0 ? (
          <SettingsRow
            label={t(
              "Nothing in quick access yet. Add destinations, agents or crews below.",
            )}
            disabled
          />
        ) : (
          rows.map(({ item, index, entry }) => {
            const name =
              entry?.name ??
              (loadingSources(item)
                ? t("Loading…")
                : item.kind === "agent"
                  ? t("Agent unavailable")
                  : t("Crew unavailable"));
            return (
              <SettingsRow
                key={`${item.kind}:${item.id}`}
                leading={entry ? glyph(entry) : undefined}
                label={name}
                detail={
                  entry || loadingSources(item)
                    ? undefined
                    : t("Not shown: it was deleted or you no longer have access")
                }
                accessory={
                  <View style={styles.controls}>
                    <OrderButton
                      label={t("Move {{name}} up", { name })}
                      disabled={index === 0}
                      onPress={() => change(moveQuickAccess(items, index, -1))}
                    >
                      <ChevronUp size={18} color={colors.textPrimary} />
                    </OrderButton>
                    <OrderButton
                      label={t("Move {{name}} down", { name })}
                      disabled={index === items.length - 1}
                      onPress={() => change(moveQuickAccess(items, index, 1))}
                    >
                      <ChevronDown size={18} color={colors.textPrimary} />
                    </OrderButton>
                    <OrderButton
                      label={t("Remove {{name}}", { name })}
                      onPress={() => change(removeQuickAccess(items, index))}
                    >
                      <Minus size={18} color={colors.danger} />
                    </OrderButton>
                  </View>
                }
              />
            );
          })
        )}
      </SettingsGroup>
      <SettingsGroup
        title={t("Add from the menu")}
        footer={
          full
            ? t("Quick access holds up to 12 items. Remove one to add another.")
            : undefined
        }
      >
        {candidates.destinations.map((id) => {
          const name = quickAccessDestinationLabel(id);
          return (
            <AddRow
              key={id}
              name={name}
              leading={glyph({
                key: id,
                item: { kind: "destination", id },
                kind: "destination",
                id,
                name,
                label: name,
              })}
              disabled={!editable || full}
              onAdd={() => change(addQuickAccess(items, { kind: "destination", id }))}
            />
          );
        })}
      </SettingsGroup>
      {candidates.agents.length ? (
        <SettingsGroup title={t("Add an agent")}>
          {candidates.agents.map((agent) => {
            const name = chatAgentLabel(agent);
            return (
              <AddRow
                key={agent.id}
                name={name}
                leading={glyph({
                  key: agent.id,
                  item: { kind: "agent", id: agent.id },
                  kind: "agent",
                  id: agent.id,
                  name,
                  label: name,
                  agent,
                })}
                disabled={!editable || full}
                onAdd={() =>
                  change(addQuickAccess(items, { kind: "agent", id: agent.id }))
                }
              />
            );
          })}
        </SettingsGroup>
      ) : null}
      {candidates.crews.length ? (
        <SettingsGroup title={t("Add a crew")}>
          {candidates.crews.map((crew) => (
            <AddRow
              key={crew.id}
              name={crew.name}
              leading={glyph({
                key: crew.id,
                item: { kind: "crew", id: crew.id },
                kind: "crew",
                id: crew.id,
                name: crew.name,
                label: crew.name,
                members: crew.members,
              })}
              disabled={!editable || full}
              onAdd={() =>
                change(addQuickAccess(items, { kind: "crew", id: crew.id }))
              }
            />
          ))}
        </SettingsGroup>
      ) : null}
      <SettingsGroup
        footer={t(
          "Puts New chat, Notifications, Scan QR code, Recent chats and Settings back.",
        )}
      >
        <SettingsRow
          label={t("Reset to default")}
          tone="action"
          disabled={!editable || !customized}
          onPress={() => {
            quickAccess.clearSaveError();
            quickAccess.reset();
          }}
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

/** A destination, agent or crew that can be added: the whole row adds it. */
function AddRow({
  name,
  leading,
  disabled,
  onAdd,
}: {
  name: string;
  leading: ReactNode;
  disabled: boolean;
  onAdd: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  return (
    <SettingsRow
      leading={leading}
      label={name}
      accessibilityLabel={t("Add {{name}}", { name })}
      disabled={disabled}
      onPress={onAdd}
      accessory={<Plus size={18} color={disabled ? colors.textMuted : colors.primary} />}
    />
  );
}

/** Up, down or remove: a 44pt icon target with a complete spoken label. */
function OrderButton({
  label,
  disabled = false,
  onPress,
  children,
}: {
  label: string;
  disabled?: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.order,
        disabled && styles.disabled,
        pressed && !disabled && styles.pressed,
      ]}
    >
      {children}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  controls: { flexDirection: "row", alignItems: "center" },
  order: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  disabled: { opacity: 0.3 },
  pressed: { opacity: 0.6 },
});
