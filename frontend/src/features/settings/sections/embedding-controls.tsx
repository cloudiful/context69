import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

/**
 * The embedding model block.
 *
 * The document model is the identity every stored vector was built with; the
 * optional query model inherits it when blank. The manual request limits are
 * editable.
 */
export function EmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft } = page;
  const batching = runtimeDraft.embedding.batching;
  const inputPolicy = runtimeDraft.embedding.input_policy;
  const tokenizerArtifacts = page.runtimeQuery.data?.tokenizer_artifacts ?? [];
  const maxInputEnabled = inputPolicy.max_input_tokens != null;

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

        <div className="grid gap-3 rounded-md border border-border/60 p-3">
          <p className="text-sm font-medium">{t("settings.runtime.embeddingInputPolicyTitle")}</p>
          <FormField
            label={t("settings.runtime.embeddingInputTokenizer")}
            htmlFor="runtime-embedding-input-tokenizer"
            helper={t("settings.runtime.embeddingInputTokenizerHelper")}
          >
            <Select
              value={inputPolicy.tokenizer}
              onValueChange={(value) => updateRuntimeDraft((draft) => {
                draft.embedding.input_policy.tokenizer =
                  value as typeof draft.embedding.input_policy.tokenizer;
                if (value !== "hf_local") {
                  draft.embedding.input_policy.tokenizer_sha256 = null;
                  draft.embedding.input_policy.add_special_tokens = false;
                }
              })}
            >
              <SelectTrigger
                id="runtime-embedding-input-tokenizer"
                aria-label={t("settings.runtime.embeddingInputTokenizer")}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="cl100k_base">cl100k_base</SelectItem>
                <SelectItem value="o200k_base">o200k_base</SelectItem>
                <SelectItem value="hf_local">
                  {t("settings.runtime.chunkingTokenizerHfLocal")}
                </SelectItem>
              </SelectContent>
            </Select>
          </FormField>
          {inputPolicy.tokenizer === "hf_local" ? (
            tokenizerArtifacts.length > 0 ? (
              <FormField
                label={t("settings.runtime.embeddingInputArtifact")}
                htmlFor="runtime-embedding-input-artifact"
                helper={t("settings.runtime.embeddingInputArtifactHelper")}
              >
                <Select
                  value={inputPolicy.tokenizer_sha256 ?? undefined}
                  onValueChange={(value) => updateRuntimeDraft((draft) => {
                    draft.embedding.input_policy.tokenizer_sha256 = value;
                  })}
                >
                  <SelectTrigger
                    id="runtime-embedding-input-artifact"
                    aria-label={t("settings.runtime.embeddingInputArtifact")}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {tokenizerArtifacts.map((artifact) => (
                      <SelectItem key={artifact.sha256} value={artifact.sha256}>
                        {`${artifact.sha256.slice(0, 12)}… (${artifact.size_bytes} B)`}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </FormField>
            ) : (
              <p
                data-testid="runtime-embedding-input-artifact-empty"
                className="text-xs text-muted-foreground"
              >
                {t("settings.runtime.chunkingTokenizerArtifactEmpty")}
              </p>
            )
          ) : null}
          <ToggleField
            id="runtime-embedding-input-limit"
            testId="runtime-embedding-input-limit"
            label={t("settings.runtime.embeddingInputLimit")}
            helper={t("settings.runtime.embeddingInputLimitHelper")}
            checked={maxInputEnabled}
            onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
              draft.embedding.input_policy.max_input_tokens = checked ? 8192 : null;
            })}
          />
          {maxInputEnabled ? (
            <NumberField
              id="runtime-embedding-input-max-tokens"
              label={t("settings.runtime.embeddingInputMaxTokens")}
              min={1}
              value={inputPolicy.max_input_tokens ?? 8192}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.embedding.input_policy.max_input_tokens = value;
              })}
            />
          ) : null}
          <TextField
            id="runtime-embedding-document-prefix"
            label={t("settings.runtime.embeddingDocumentPrefix")}
            helper={t("settings.runtime.embeddingServerPrefixHelper")}
            value={inputPolicy.document_server_prefix}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.embedding.input_policy.document_server_prefix = value;
            })}
          />
          <TextField
            id="runtime-embedding-query-prefix"
            label={t("settings.runtime.embeddingQueryPrefix")}
            helper={t("settings.runtime.embeddingServerPrefixHelper")}
            value={inputPolicy.query_server_prefix}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.embedding.input_policy.query_server_prefix = value;
            })}
          />
          {inputPolicy.tokenizer === "hf_local" ? (
            <ToggleField
              id="runtime-embedding-input-special"
              testId="runtime-embedding-input-special"
              label={t("settings.runtime.embeddingInputSpecial")}
              helper={t("settings.runtime.embeddingInputSpecialHelper")}
              checked={inputPolicy.add_special_tokens}
              onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
                draft.embedding.input_policy.add_special_tokens = checked;
              })}
            />
          ) : null}
          <p className="text-xs text-muted-foreground">
            {t("settings.runtime.embeddingInputPolicyHelper")}
          </p>
        </div>

        <div className="grid gap-3 rounded-md border border-border/60 p-3">
          <p className="text-sm font-medium">{t("settings.runtime.embeddingBatchingTitle")}</p>
          <NumberField
            id="runtime-embedding-batch-items"
            label={t("settings.runtime.embeddingBatchItems")}
            min={1}
            value={batching.max_items_per_request}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.embedding.batching.max_items_per_request = value;
            })}
          />
          <details className="rounded-md border border-border/60 p-2">
            <summary className="cursor-pointer text-sm">
              {t("settings.runtime.embeddingBatchingAdvanced")}
            </summary>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <NumberField
                id="runtime-embedding-batch-item-chars"
                label={t("settings.runtime.embeddingBatchItemChars")}
                min={1}
                value={batching.max_item_chars}
                onChange={(value) => updateRuntimeDraft((draft) => {
                  draft.embedding.batching.max_item_chars = value;
                })}
              />
              <NumberField
                id="runtime-embedding-batch-request-chars"
                label={t("settings.runtime.embeddingBatchRequestChars")}
                min={1}
                value={batching.max_request_chars}
                onChange={(value) => updateRuntimeDraft((draft) => {
                  draft.embedding.batching.max_request_chars = value;
                })}
              />
              <NumberField
                id="runtime-embedding-batch-request-bytes"
                label={t("settings.runtime.embeddingBatchRequestBytes")}
                min={1}
                value={batching.max_request_bytes}
                onChange={(value) => updateRuntimeDraft((draft) => {
                  draft.embedding.batching.max_request_bytes = value;
                })}
              />
            </div>
          </details>
          <p className="text-xs text-muted-foreground">
            {t("settings.runtime.embeddingBatchingHelper")}
          </p>
        </div>
      </div>
    </SettingsBlock>
  );
}
