import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { useSettings } from "@/features/settings/settings-context";
import type { EmbeddingCapabilitiesResponse } from "@/services/api";

import { TestConnectionButton } from "./test-connection-button";

/**
 * The embedding model block.
 *
 * The document model is the identity every stored vector was built with; the
 * optional query model inherits it when blank. The manual request limits are
 * editable, and the provider's own advertised facts are shown read-only beside
 * them: they never overwrite a draft and never activate anything.
 */
export function EmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft, embeddingCapabilities } = page;
  const batching = runtimeDraft.embedding.batching;

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
          <p className="text-sm font-medium">{t("settings.runtime.embeddingBatchingTitle")}</p>
          <div className="grid gap-3 sm:grid-cols-2">
            <NumberField
              id="runtime-embedding-batch-items"
              label={t("settings.runtime.embeddingBatchItems")}
              min={1}
              value={batching.max_items_per_request}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.embedding.batching.max_items_per_request = value;
              })}
            />
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
          <p className="text-xs text-muted-foreground">
            {t("settings.runtime.embeddingBatchingHelper")}
          </p>
        </div>

        {embeddingCapabilities ? (
          <EmbeddingCapabilitiesSummary capabilities={embeddingCapabilities} />
        ) : null}
      </div>
    </SettingsBlock>
  );
}

/**
 * The provider's own advertised facts, read-only.
 *
 * Every value is what the provider published, never a locally computed guess:
 * the per-input token limit is enforced by the provider, and its execution
 * batch size is server-side metadata rather than a client request limit.
 */
function EmbeddingCapabilitiesSummary({
  capabilities,
}: {
  capabilities: EmbeddingCapabilitiesResponse;
}) {
  const { t } = useTranslation();
  const aliases = capabilities.role_aliases
    ? Object.entries(capabilities.role_aliases).map(([alias, role]) => `${alias} (${role})`).join(", ")
    : "";
  const dimensions = capabilities.supported_dimensions?.length
    ? capabilities.supported_dimensions.join(", ")
    : "";
  const effectiveItems = capabilities.effective_limits.max_items_per_request;
  const manualItems = capabilities.manual_limits.max_items_per_request;

  return (
    <div
      data-testid="runtime-embedding-capabilities"
      className="grid gap-1 rounded-md border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground"
    >
      <p className="text-sm font-medium text-foreground">
        {t("settings.runtime.embeddingCapabilitiesTitle")}
      </p>
      <p>{t("settings.runtime.embeddingCapabilitiesSource", { source: capabilities.source })}</p>
      {capabilities.source === "unknown" ? (
        <p>{t("settings.runtime.embeddingCapabilitiesUnknown")}</p>
      ) : null}
      {capabilities.reason ? (
        <p>{t("settings.runtime.embeddingCapabilitiesReason", { reason: capabilities.reason })}</p>
      ) : null}
      {capabilities.advertised_model ? (
        <p>{t("settings.runtime.embeddingCapabilitiesModel", { model: capabilities.advertised_model })}</p>
      ) : null}
      {dimensions ? (
        <p>{t("settings.runtime.embeddingCapabilitiesDimensions", { dimensions })}</p>
      ) : null}
      {aliases ? (
        <p>{t("settings.runtime.embeddingCapabilitiesAliases", { aliases })}</p>
      ) : null}
      <p>
        {t("settings.runtime.embeddingCapabilitiesEffectiveItems", {
          effective: effectiveItems,
          manual: manualItems,
        })}
      </p>
      {capabilities.advertised_tokens_per_input_limit ? (
        <p>
          {t("settings.runtime.embeddingCapabilitiesTokenLimit", {
            limit: capabilities.advertised_tokens_per_input_limit,
          })}
        </p>
      ) : (
        <p>{t("settings.runtime.embeddingCapabilitiesNoTokenLimit")}</p>
      )}
      {capabilities.advertised_tokens_per_input_limit
        && (capabilities.tokens_include_role_prefix || capabilities.tokens_include_special_tokens) ? (
        <p>{t("settings.runtime.embeddingCapabilitiesTokenLimitIncludes")}</p>
      ) : null}
      {capabilities.internal_batch_size ? (
        <p>
          {t("settings.runtime.embeddingCapabilitiesInternalBatch", {
            size: capabilities.internal_batch_size,
          })}
        </p>
      ) : null}
    </div>
  );
}
