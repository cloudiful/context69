import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

/**
 * The optional Git code embedding model block.
 *
 * It is independent of the document model: enabling it never borrows the
 * document endpoint, model, or credential. Details are only edited while
 * enabled, but the draft keeps them when the block is turned off so a later
 * save can re-enable the same configuration.
 */
export function CodeEmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateCodeEmbeddingDraft } = page;
  const code = runtimeDraft.code_embedding;

  return (
    <SettingsBlock
      id="settings-code-embedding"
      compact
      title={t("settings.runtime.codeEmbeddingTitle")}
      actions={
        <TestConnectionButton
          testing={page.codeEmbeddingTesting}
          disabled={!code.enabled || !code.base_url.trim() || !code.model.trim()}
          testId="runtime-code-embedding-test"
          label={t("settings.runtime.codeEmbeddingTest")}
          onTest={() => void page.testCodeEmbeddingConnection()}
        />
      }
    >
      <div className="grid max-w-2xl gap-3">
        <ToggleField
          id="runtime-code-embedding-enabled"
          testId="runtime-code-embedding-enabled"
          label={t("settings.runtime.codeEmbeddingEnabled")}
          helper={t("settings.runtime.codeEmbeddingEnabledHelper")}
          checked={code.enabled}
          onCheckedChange={(checked) => updateCodeEmbeddingDraft((draft) => {
            draft.enabled = checked;
          })}
        />
        {code.enabled ? (
          <>
            <TextField
              id="runtime-code-embedding-base-url"
              type="url"
              label={t("settings.runtime.codeEmbeddingBaseUrl")}
              placeholder="https://openrouter.ai/api/v1"
              value={code.base_url}
              onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                draft.base_url = value;
              })}
            />
            <TextField
              id="runtime-code-embedding-api-key"
              type="password"
              autoComplete="new-password"
              label={t("settings.runtime.codeEmbeddingApiKey")}
              placeholder={code.has_api_key ? t("settings.runtime.codeEmbeddingKeyStored") : "sk-..."}
              value={code.api_key}
              onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                draft.api_key = value;
                draft.clear_api_key = false;
              })}
            />
            {code.has_api_key ? (
              <ToggleField
                id="runtime-code-embedding-clear-key"
                testId="runtime-code-embedding-clear-key"
                label={t("settings.runtime.codeEmbeddingClearKey")}
                helper={t("settings.runtime.codeEmbeddingClearKeyHelper")}
                checked={code.clear_api_key}
                onCheckedChange={(checked) => updateCodeEmbeddingDraft((draft) => {
                  draft.clear_api_key = checked;
                })}
              />
            ) : null}
            <TextField
              id="runtime-code-embedding-model"
              label={t("settings.runtime.codeEmbeddingModel")}
              helper={t("settings.runtime.codeEmbeddingModelHelper")}
              value={code.model}
              onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                draft.model = value;
              })}
            />
            <TextField
              id="runtime-code-embedding-query-model"
              label={t("settings.runtime.codeEmbeddingQueryModel")}
              helper={t("settings.runtime.codeEmbeddingQueryModelHelper")}
              value={code.query_model}
              onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                draft.query_model = value;
              })}
            />
            <div className="grid gap-3 sm:grid-cols-2">
              <NumberField
                id="runtime-code-embedding-dimensions"
                label={t("settings.runtime.codeEmbeddingDimensions")}
                min={1}
                value={code.dimensions}
                onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                  draft.dimensions = value;
                })}
              />
              <NumberField
                id="runtime-code-embedding-timeout"
                label={t("settings.runtime.codeEmbeddingTimeout")}
                min={1}
                value={code.timeout_secs}
                onChange={(value) => updateCodeEmbeddingDraft((draft) => {
                  draft.timeout_secs = value;
                })}
              />
            </div>
          </>
        ) : null}
      </div>
    </SettingsBlock>
  );
}
