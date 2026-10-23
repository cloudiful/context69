import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";

export function SearchSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { searchDraft, setRerankApiKeyDraft, rerankApiKeyDraft, updateSearchDraft } = page;

  const boostMargin = Math.max(
    0,
    1 - Number(searchDraft.vector_weight ?? 0) - Number(searchDraft.keyword_weight ?? 0),
  ).toFixed(2);

  return (
    <SettingsSection title={t("settings.search.title")}>
      <SettingsGate>
        <div id="settings-search" className="grid gap-3">
        <SettingsBlock compact>
          <div className="grid gap-3">
            <div className="grid max-w-2xl gap-3">
              <FormField label={t("settings.search.mode")} htmlFor="search-mode">
                <Select
                  value={searchDraft.mode}
                  onValueChange={(value) => updateSearchDraft((draft) => {
                    draft.mode = value as typeof draft.mode;
                  })}
                >
                  <SelectTrigger id="search-mode" data-testid="search-mode" aria-label={t("settings.search.mode")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="hybrid">{t("settings.search.modeHybrid")}</SelectItem>
                    <SelectItem value="vector">{t("settings.search.modeVector")}</SelectItem>
                  </SelectContent>
                </Select>
              </FormField>

              <NumberField
                id="search-candidate-limit"
                testId="search-candidate-limit"
                label={t("settings.search.candidateLimit")}
                value={searchDraft.candidate_limit}
                min={1}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.candidate_limit = value;
                })}
              />

              <NumberField
                id="search-rerank-timeout"
                testId="search-rerank-timeout"
                label={t("settings.search.timeout")}
                value={searchDraft.timeout_secs}
                min={1}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.timeout_secs = value;
                })}
              />
            </div>

            <ToggleField
              id="search-rerank-enabled"
              testId="search-rerank-enabled"
              label={t("settings.search.rerankEnabled")}
              checked={searchDraft.rerank_enabled}
              onCheckedChange={(checked) => updateSearchDraft((draft) => {
                draft.rerank_enabled = checked;
              })}
            />

            <div className="grid max-w-2xl gap-3">
              <TextField
                id="search-rerank-base-url"
                testId="search-rerank-base-url"
                type="url"
                label={t("settings.search.rerankBaseUrl")}
                placeholder="https://openrouter.ai/api/v1"
                value={searchDraft.rerank_base_url}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.rerank_base_url = value;
                })}
              />

              <TextField
                id="search-rerank-model"
                testId="search-rerank-model"
                label={t("settings.search.rerankModel")}
                placeholder="cohere/rerank-4-fast"
                value={searchDraft.rerank_model}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.rerank_model = value;
                })}
              />

              <TextField
                id="search-rerank-api-key"
                testId="search-rerank-api-key"
                label={t("settings.search.rerankApiKey")}
                type="password"
                autoComplete="new-password"
                placeholder="sk-or-..."
                value={rerankApiKeyDraft}
                onChange={setRerankApiKeyDraft}
              />
            </div>

            <div className="grid max-w-2xl gap-3">
              <NumberField
                id="search-vector-weight"
                testId="search-vector-weight"
                label={t("settings.search.vectorWeight")}
                helper={t("settings.search.fusionWeightsHint", { margin: boostMargin })}
                value={searchDraft.vector_weight}
                min={0}
                max={1}
                step={0.05}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.vector_weight = value;
                })}
              />

              <NumberField
                id="search-keyword-weight"
                testId="search-keyword-weight"
                label={t("settings.search.keywordWeight")}
                value={searchDraft.keyword_weight}
                min={0}
                max={1}
                step={0.05}
                onChange={(value) => updateSearchDraft((draft) => {
                  draft.keyword_weight = value;
                })}
              />
            </div>
          </div>
        </SettingsBlock>
        </div>
      </SettingsGate>
    </SettingsSection>
  );
}
