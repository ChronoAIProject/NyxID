import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { View } from "react-native";
import {
  Card,
  Label,
  ActionButton,
} from "../../components/workspace/WorkspaceUI";
import { groupCanDecide, groupPendingReview } from "./groupState";
import type { GroupPendingAction } from "./groupContracts";

export function GroupPendingCard({
  action,
  name,
  actorId,
  organization,
  blocked,
  onReview,
  onDecision,
  onAnswer,
}: {
  action: GroupPendingAction;
  name: string;
  actorId?: string;
  organization: boolean;
  blocked: boolean;
  onReview?: () => void;
  onDecision: (decision: "allow" | "deny") => void;
  onAnswer: (yes: boolean) => void;
}) {
  useAppLanguage();
  const expired = Date.parse(action.expires_at) <= Date.now();
  const review = groupPendingReview(action, actorId, organization);
  const canDecide = groupCanDecide(action, actorId, organization);
  return (
    <Card>
      <View
        accessibilityLabel={translateApp("Confirm: {{summary}}", {
          summary: action.summary,
        })}
        accessibilityLiveRegion="polite"
      >
        <Label>
          {translateApp("{{name}} wants to: {{summary}}", { name: name, summary: action.summary})}
        </Label>
        {expired ? (
          <Label muted>
            {translateApp(
              "This request expired. Refresh the crew for its latest state.",
            )}
          </Label>
        ) : action.authored_skill ? (
          review && onReview ? (
            <ActionButton
              compact
              label={translateApp("Review draft")}
              variant="secondary"
              disabled={blocked}
              onPress={onReview}
            />
          ) : (
            <Label muted>{translateApp("Awaiting {{name}} review", { name: action.triggering_person?.display_name ?? translateApp("owner") })}</Label>
          )
        ) : action.triggering_person ? (
          <>
            <Label muted>{translateApp("Awaiting {{name}}", { name: action.triggering_person.display_name })}
            </Label>
            {canDecide ? (
              <View style={{ flexDirection: "row", gap: 8 }}>
                <ActionButton
                  compact
                  label={translateApp("Deny")}
                  variant="secondary"
                  disabled={blocked}
                  onPress={() => onDecision("deny")}
                />
                <ActionButton
                  compact
                  label={translateApp("Allow")}
                  disabled={blocked}
                  onPress={() => onDecision("allow")}
                />
              </View>
            ) : null}
          </>
        ) : action.confirm_phrase && !organization ? (
          <View style={{ flexDirection: "row", gap: 8 }}>
            <ActionButton
              compact
              label={translateApp("Cancel")}
              variant="secondary"
              disabled={blocked}
              onPress={() => onAnswer(false)}
            />
            <ActionButton
              compact
              label={translateApp("Confirm")}
              disabled={blocked}
              onPress={() => onAnswer(true)}
            />
          </View>
        ) : (
          <Label muted>
            {translateApp("Awaiting the owner in the agent's conversation.")}
          </Label>
        )}
      </View>
    </Card>
  );
}
