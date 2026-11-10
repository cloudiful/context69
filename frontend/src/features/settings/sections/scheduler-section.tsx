import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

export function SchedulerSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft } = page;

  return (
    <SettingsSection title={t("settings.sections.scheduler")}>
      <SettingsGate>
        <SettingsBlock
          id="settings-scheduler"
          compact
          title={t("settings.runtime.schedulerTitle")}
          actions={
            <TestConnectionButton
              testing={page.valkeyTesting}
              disabled={!runtimeDraft.scheduler.valkey_url.trim()}
              testId="runtime-valkey-test"
              label={t("settings.runtime.valkeyTest")}
              onTest={() => void page.testValkeyConnection()}
            />
          }
        >
          <div className="grid max-w-2xl gap-3">
            <NumberField
              id="runtime-scheduler-interval"
              label={t("settings.runtime.schedulerInterval")}
              min={1}
              value={runtimeDraft.scheduler.interval_secs}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.interval_secs = value;
              })}
            />
            <NumberField
              id="runtime-scheduler-concurrency"
              label={t("settings.runtime.schedulerConcurrency")}
              min={1}
              value={runtimeDraft.scheduler.max_concurrency}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.max_concurrency = value;
              })}
            />
            <TextField
              id="runtime-scheduler-job-id"
              label={t("settings.runtime.schedulerJobId")}
              value={runtimeDraft.scheduler.job_id}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.job_id = value;
              })}
            />
            <TextField
              id="runtime-scheduler-valkey-url"
              label={t("settings.runtime.schedulerValkeyUrl")}
              helper={t("settings.runtime.valkeyRestartRequired")}
              placeholder="redis://valkey:6379/0"
              value={runtimeDraft.scheduler.valkey_url}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.valkey_url = value;
              })}
            />
            <ToggleField
              id="runtime-scheduler-run-on-start"
              testId="runtime-scheduler-run-on-start"
              label={t("settings.runtime.runOnStart")}
              checked={runtimeDraft.scheduler.run_on_start}
              onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
                draft.scheduler.run_on_start = checked;
              })}
            />
          </div>
        </SettingsBlock>
      </SettingsGate>
    </SettingsSection>
  );
}
