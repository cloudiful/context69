import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

/**
 * The embedding model block.
 *
 * The document model is the identity every stored vector was built with; the
 * optional query model inherits it when blank.
 */
export function EmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft } = page;

  return (
    <SettingsBlock
      id="settings-embedding"
      compact
      title={t("settings.runtime.embeddingTitle")}
      actions={
        <TestConnectionButton
          testing={page.embeddingTesting}
          testId="runtime-embedding-test"
          label={t("settings.runtime.embeddingTest")}
          onTest={() => void page.testEmbeddingConnection()}
        />
      }
    >
      <div className="grid max-w-2xl gap-3">
        <TextField
          id="runtime-embedding-base-url"
          type="url"
          label={t("settings.runtime.embeddingBaseUrl")}
          placeholder="https://openrouter.ai/api/v1"
          value={runtimeDraft.embedding.base_url}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.base_url = value;
          })}
        />
        <TextField
          id="runtime-embedding-api-key"
          type="password"
          autoComplete="new-password"
          label={t("settings.runtime.embeddingApiKey")}
          placeholder="sk-..."
          value={runtimeDraft.embedding.api_key}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.api_key = value;
          })}
        />
        <TextField
          id="runtime-embedding-model"
          label={t("settings.runtime.embeddingDocumentModel")}
          helper={t("settings.runtime.embeddingDocumentModelHelper")}
          value={runtimeDraft.embedding.model}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.model = value;
          })}
        />
        <TextField
          id="runtime-embedding-query-model"
          label={t("settings.runtime.embeddingQueryModel")}
          helper={t("settings.runtime.embeddingQueryModelHelper")}
          value={runtimeDraft.embedding.query_model}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.query_model = value;
          })}
        />
        <NumberField
          id="runtime-embedding-dimensions"
          label={t("settings.runtime.embeddingDimensions")}
          min={1}
          value={runtimeDraft.embedding.dimensions}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.dimensions = value;
          })}
        />
        <NumberField
          id="runtime-embedding-timeout"
          label={t("settings.runtime.embeddingTimeout")}
          min={1}
          value={runtimeDraft.embedding.timeout_secs}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.embedding.timeout_secs = value;
          })}
        />
      </div>
    </SettingsBlock>
  );
}
