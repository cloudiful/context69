import { useTranslation } from "react-i18next";

import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { useSettings } from "@/features/settings/settings-context";
import { DEFAULT_IMAGE_COLLECTION_NAME } from "@/utils/settings";

import { OptionalNumberField } from "./code-embedding-controls";
import { TestConnectionButton } from "./test-connection-button";

/**
 * The optional image embedding model block.
 *
 * Every provider field is an override: a blank one inherits the document
 * configuration field by field, and the block shows the inherited value as
 * the placeholder. The collection name is a namespace, not a physical
 * collection — the deployment derives the physical collection from its own
 * runtime fingerprint. Details are only edited while enabled, but the draft
 * keeps them when the block is turned off so a later save can re-enable the
 * same configuration.
 */
export function ImageEmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateImageEmbeddingDraft } = page;
  const image = runtimeDraft.image_embedding;
  const document = runtimeDraft.embedding;
  // The effective image model is the document model while no image model is
  // stored, so a document model change moves the image identity with it.
  const inheritsModel = !image.model.trim();
  // The blank query field resolves to the document query model, then to the
  // effective image model — the same order the save and the probe use.
  const effectiveModel = image.model.trim() || document.model.trim();
  const inheritedQueryModel = document.query_model.trim() || effectiveModel;
  // Inherited dimensions move the vector identity with the document too.
  const inheritsDimensions = image.dimensions === null;

  return (
    <SettingsBlock
      id="settings-image-embedding"
      compact
      title={t("settings.runtime.imageEmbeddingTitle")}
      actions={
        <TestConnectionButton
          testing={page.imageEmbeddingTesting}
          disabled={!image.enabled}
          testId="runtime-image-embedding-test"
          label={t("settings.runtime.imageEmbeddingTest")}
          onTest={() => void page.testImageEmbeddingConnection()}
        />
      }
    >
      <div className="grid max-w-3xl gap-3">
        <ToggleField
          id="runtime-image-embedding-enabled"
          testId="runtime-image-embedding-enabled"
          layout="row"
          label={t("settings.runtime.imageEmbeddingEnabled")}
          helper={t("settings.runtime.imageEmbeddingEnabledHelper")}
          checked={image.enabled}
          onCheckedChange={(checked) => updateImageEmbeddingDraft({ enabled: checked })}
        />
        {image.enabled ? (
          <>
            <TextField
              id="runtime-image-embedding-base-url"
              type="url"
              layout="row"
              controlClassName="max-w-xl"
              label={t("settings.runtime.imageEmbeddingBaseUrl")}
              helper={t("settings.runtime.imageEmbeddingBaseUrlHelper")}
              placeholder={document.base_url.trim() || "https://openrouter.ai/api/v1"}
              value={image.base_url}
              onChange={(value) => updateImageEmbeddingDraft({ base_url: value })}
            />
            <TextField
              id="runtime-image-embedding-api-key"
              type="password"
              autoComplete="new-password"
              layout="row"
              label={t("settings.runtime.imageEmbeddingApiKey")}
              helper={t("settings.runtime.imageEmbeddingApiKeyHelper")}
              placeholder={image.has_api_key ? t("settings.runtime.imageEmbeddingKeyStored") : "sk-..."}
              value={image.api_key}
              onChange={(value) => updateImageEmbeddingDraft({ api_key: value })}
            />
            {image.has_api_key ? (
              <ToggleField
                id="runtime-image-embedding-clear-key"
                testId="runtime-image-embedding-clear-key"
                layout="row"
                label={t("settings.runtime.imageEmbeddingClearKey")}
                helper={t("settings.runtime.imageEmbeddingClearKeyHelper")}
                checked={image.clear_api_key}
                onCheckedChange={(checked) => updateImageEmbeddingDraft({ clear_api_key: checked })}
              />
            ) : null}
            {image.inherits_document_api_key ? (
              <p className="text-xs text-muted-foreground" data-testid="runtime-image-embedding-inherited-key">
                {t("settings.runtime.imageEmbeddingInheritedCredential")}
              </p>
            ) : null}
            <TextField
              id="runtime-image-embedding-model"
              layout="row"
              label={t("settings.runtime.imageEmbeddingModel")}
              helper={t("settings.runtime.imageEmbeddingModelHelper")}
              placeholder={document.model.trim() || undefined}
              value={image.model}
              onChange={(value) => updateImageEmbeddingDraft({ model: value })}
            />
            <TextField
              id="runtime-image-embedding-query-model"
              layout="row"
              label={t("settings.runtime.imageEmbeddingQueryModel")}
              helper={t("settings.runtime.imageEmbeddingQueryModelHelper")}
              placeholder={inheritedQueryModel || undefined}
              value={image.query_model}
              onChange={(value) => updateImageEmbeddingDraft({ query_model: value })}
            />
            <TextField
              id="runtime-image-embedding-collection"
              layout="row"
              label={t("settings.runtime.imageEmbeddingCollection")}
              helper={t("settings.runtime.imageEmbeddingCollectionHelper")}
              placeholder={DEFAULT_IMAGE_COLLECTION_NAME}
              value={image.collection_name}
              onChange={(value) => updateImageEmbeddingDraft({ collection_name: value })}
            />
            <OptionalNumberField
              id="runtime-image-embedding-dimensions"
              layout="row"
              label={t("settings.runtime.imageEmbeddingDimensions")}
              helper={t("settings.runtime.imageEmbeddingDimensionsHelper")}
              min={1}
              placeholder={String(document.dimensions)}
              value={image.dimensions}
              onChange={(value) => updateImageEmbeddingDraft({ dimensions: value })}
            />
            <OptionalNumberField
              id="runtime-image-embedding-timeout"
              layout="row"
              label={t("settings.runtime.imageEmbeddingTimeout")}
              helper={t("settings.runtime.imageEmbeddingTimeoutHelper")}
              min={1}
              placeholder={String(document.timeout_secs)}
              value={image.timeout_secs}
              onChange={(value) => updateImageEmbeddingDraft({ timeout_secs: value })}
            />
            {inheritsModel || inheritsDimensions ? (
              <p className="text-xs text-muted-foreground" data-testid="runtime-image-embedding-inherited-identity">
                {t("settings.runtime.imageEmbeddingInheritedIdentityWarning")}
              </p>
            ) : null}
            <p className="text-xs text-muted-foreground" data-testid="runtime-image-embedding-test-scope">
              {t("settings.runtime.imageEmbeddingTestScope")}
            </p>
          </>
        ) : null}
      </div>
    </SettingsBlock>
  );
}
