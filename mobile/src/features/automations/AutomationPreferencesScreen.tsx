import { useState } from "react";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { RootStackParamList } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { automationsApi } from "../../lib/api/automationsApi";
import {
  automationPreferencesSchema,
  type AutomationSettings,
} from "./automationContracts";
import { deviceTimezone } from "./automationTime";
import {
  AUTOMATIONS,
  AUTOMATION_SETTINGS,
  AutomationLoading,
  automationError,
  useAutomationRuntime,
  useAutomationWrite,
} from "./AutomationUI";

export function AutomationPreferencesScreen({
  navigation,
}: NativeStackScreenProps<RootStackParamList, "AutomationPreferences">) {
  const runtime = useAutomationRuntime();
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled: runtime.active,
    retry: false,
  });
  return (
    <WorkspacePage title="Automation preferences" showHeading={false}>
      {settings.isPending && <AutomationLoading />}
      {settings.error && (
        <>
          <Notice tone="error" message={automationError(settings.error)} />
          <ActionButton
            label="Retry preferences"
            variant="secondary"
            onPress={() => void settings.refetch()}
          />
        </>
      )}
      {settings.data && (
        <PreferencesForm settings={settings.data} onBack={navigation.goBack} />
      )}
    </WorkspacePage>
  );
}

function PreferencesForm({
  settings,
  onBack,
}: {
  settings: AutomationSettings;
  onBack: () => void;
}) {
  const client = useQueryClient();
  const write = useAutomationWrite();
  const [timezone, setTimezone] = useState(
    settings.timezone ?? deviceTimezone(),
  );
  const [minimum, setMinimum] = useState(
    String(settings.schedule_minimum_minutes),
  );
  const [hourly, setHourly] = useState(String(settings.trigger_runs_per_hour));
  const [daily, setDaily] = useState(String(settings.trigger_runs_per_day));
  const [saved, setSaved] = useState(false);
  useWorkspaceBack({
    active: write.busy,
    defaultRole: "back",
    disabled: write.busy,
    onBack,
  });
  async function save() {
    await write.run(async (current) => {
      const values = automationPreferencesSchema.parse({
        timezone,
        schedule_minimum_minutes: Number(minimum),
        trigger_runs_per_hour: Number(hourly),
        trigger_runs_per_day: Number(daily),
      });
      const result = await automationsApi.updatePreferences(values);
      if (!current()) return;
      client.setQueryData(AUTOMATION_SETTINGS, result);
      void client.invalidateQueries({ queryKey: AUTOMATIONS });
      setSaved(true);
    });
  }
  const edit = (setter: (value: string) => void) => (value: string) => {
    setter(value);
    setSaved(false);
  };
  return (
    <>
      <Card>
        <Label>Scheduling limits</Label>
        <Label muted>
          Budgets defer runs when exhausted. They do not turn off your
          automations. These settings apply to your personal account.
        </Label>
      </Card>
      {write.error && <Notice tone="error" message={write.error} />}
      {saved && (
        <Notice tone="success" message="Automation preferences saved." />
      )}
      <Field
        label="Timezone"
        value={timezone}
        onChangeText={edit(setTimezone)}
        editable={!write.busy}
        placeholder="Asia/Shanghai"
      />
      <ActionButton
        label="Use device timezone"
        variant="secondary"
        disabled={write.busy}
        onPress={() => {
          setTimezone(deviceTimezone());
          setSaved(false);
        }}
      />
      <Label muted>Existing calendar schedules keep their own timezone.</Label>
      <Field
        label="Minimum interval (minutes, 5–1440)"
        keyboardType="number-pad"
        value={minimum}
        onChangeText={edit(setMinimum)}
        editable={!write.busy}
      />
      <Field
        label="Runs per hour (1–300)"
        keyboardType="number-pad"
        value={hourly}
        onChangeText={edit(setHourly)}
        editable={!write.busy}
      />
      <Field
        label="Runs per day (1–3000)"
        keyboardType="number-pad"
        value={daily}
        onChangeText={edit(setDaily)}
        editable={!write.busy}
      />
      <ActionButton
        label="Save preferences"
        loading={write.busy}
        disabled={!write.online}
        onPress={() => void save()}
      />
    </>
  );
}
