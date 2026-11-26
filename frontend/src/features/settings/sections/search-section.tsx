import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsFieldRow } from "@/components/common/settings-field-row";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";

export function SearchSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { searchDraft, form } = page;

  const boostMargin = Math.max(
    0,
    1 - Number(searchDraft.vector_weight ?? 0) - Number(searchDraft.keyword_weight ?? 0),
  ).toFixed(2);

  return (
    <SettingsSection title={t("settings.search.title")}>
      <SettingsGate>
        <div id="settings-search" className="grid gap-3">
        <SettingsBlock compact>
          <div className="grid max-w-3xl gap-3">
            <SettingsFieldRow
              label={t("settings.search.mode")}
              htmlFor="search-mode"
              controlClassName="max-w-56"
            >
              <form.Field name="searchSettings.mode">
                {(field) => (
                  <Select
                    value={field.state.value}
                    onValueChange={(value) => field.handleChange(value as typeof field.state.value)}
                  >
                    <SelectTrigger id="search-mode" data-testid="search-mode" aria-label={t("settings.search.mode")}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="hybrid">{t("settings.search.modeHybrid")}</SelectItem>
                      <SelectItem value="vector">{t("settings.search.modeVector")}</SelectItem>
                    </SelectContent>
                  </Select>
                )}
              </form.Field>
            </SettingsFieldRow>

            <form.Field name="searchSettings.candidate_limit">
              {(field) => (
                <NumberField
                  id="search-candidate-limit"
                  testId="search-candidate-limit"
                  label={t("settings.search.candidateLimit")}
                  value={field.state.value}
                  min={1}
                  layout="row"
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.timeout_secs">
              {(field) => (
                <NumberField
                  id="search-rerank-timeout"
                  testId="search-rerank-timeout"
                  label={t("settings.search.timeout")}
                  value={field.state.value}
                  min={1}
                  layout="row"
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.rerank_enabled">
              {(field) => (
                <ToggleField
                  id="search-rerank-enabled"
                  testId="search-rerank-enabled"
                  layout="row"
                  label={t("settings.search.rerankEnabled")}
                  checked={field.state.value}
                  onCheckedChange={(checked) => field.handleChange(checked)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.rerank_base_url">
              {(field) => (
                <TextField
                  id="search-rerank-base-url"
                  testId="search-rerank-base-url"
                  type="url"
                  layout="row"
                  controlClassName="max-w-xl"
                  label={t("settings.search.rerankBaseUrl")}
                  placeholder="https://openrouter.ai/api/v1"
                  value={field.state.value}
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.rerank_model">
              {(field) => (
                <TextField
                  id="search-rerank-model"
                  testId="search-rerank-model"
                  layout="row"
                  label={t("settings.search.rerankModel")}
                  placeholder="cohere/rerank-4-fast"
                  value={field.state.value}
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchRerankApiKey">
              {(field) => (
                <TextField
                  id="search-rerank-api-key"
                  testId="search-rerank-api-key"
                  layout="row"
                  controlClassName="max-w-xl"
                  label={t("settings.search.rerankApiKey")}
                  type="password"
                  autoComplete="new-password"
                  placeholder="sk-or-..."
                  value={field.state.value}
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.vector_weight">
              {(field) => (
                <NumberField
                  id="search-vector-weight"
                  testId="search-vector-weight"
                  layout="row"
                  label={t("settings.search.vectorWeight")}
                  helper={t("settings.search.fusionWeightsHint", { margin: boostMargin })}
                  value={field.state.value}
                  min={0}
                  max={1}
                  step={0.05}
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>

            <form.Field name="searchSettings.keyword_weight">
              {(field) => (
                <NumberField
                  id="search-keyword-weight"
                  testId="search-keyword-weight"
                  layout="row"
                  label={t("settings.search.keywordWeight")}
                  value={field.state.value}
                  min={0}
                  max={1}
                  step={0.05}
                  onChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>
          </div>
        </SettingsBlock>
        </div>
      </SettingsGate>
    </SettingsSection>
  );
}
