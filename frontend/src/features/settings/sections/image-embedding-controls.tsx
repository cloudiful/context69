import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

/**
 * The optional image embedding model block.
 *
 * It is independent of the document and code models: enabling it never
 * borrows their endpoint, model, credential, or vectors. The collection name
 * is a namespace, not a physical collection — the deployment derives the
 * physical collection from its own runtime fingerprint. Details are only
 * edited while enabled, but the draft keeps them when the block is turned off
 * so a later save can re-enable the same configuration.
 */
export function ImageEmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateImageEmbeddingDraft } = page;
  const image = runtimeDraft.image_embedding;
  // A pending key clear cannot be probed in any state: the probe would fall
  // back to the stored credential the pending save removes.
  const clearPending = image.clear_api_key;

  return (
    <SettingsBlock
      id="settings-image-embedding"
      compact
      title={t("settings.runtime.imageEmbeddingTitle")}
      actions={
        <TestConnectionButton
          testing={page.imageEmbeddingTesting}
          disabled={
            !image.enabled
            || !image.base_url.trim()
            || !image.model.trim()
            || clearPending
          }
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
              placeholder="https://openrouter.ai/api/v1"
              value={image.base_url}
              onChange={(value) => updateImageEmbeddingDraft({ base_url: value })}
            />
            <TextField
              id="runtime-image-embedding-api-key"
              type="password"
              autoComplete="new-password"
              layout="row"
              label={t("settings.runtime.imageEmbeddingApiKey")}
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
            {clearPending ? (
              <p className="text-xs text-muted-foreground" data-testid="runtime-image-embedding-clear-key-pending">
                {t("settings.runtime.imageEmbeddingTestClearKeyBlocked")}
              </p>
            ) : null}
            <TextField
              id="runtime-image-embedding-model"
              layout="row"
              label={t("settings.runtime.imageEmbeddingModel")}
              helper={t("settings.runtime.imageEmbeddingModelHelper")}
              value={image.model}
              onChange={(value) => updateImageEmbeddingDraft({ model: value })}
            />
            <TextField
              id="runtime-image-embedding-query-model"
              layout="row"
              label={t("settings.runtime.imageEmbeddingQueryModel")}
              helper={t("settings.runtime.imageEmbeddingQueryModelHelper")}
              value={image.query_model}
              onChange={(value) => updateImageEmbeddingDraft({ query_model: value })}
            />
            <TextField
              id="runtime-image-embedding-collection"
              layout="row"
              label={t("settings.runtime.imageEmbeddingCollection")}
              helper={t("settings.runtime.imageEmbeddingCollectionHelper")}
              placeholder="context69_images"
              value={image.collection_name}
              onChange={(value) => updateImageEmbeddingDraft({ collection_name: value })}
            />
            <NumberField
              id="runtime-image-embedding-dimensions"
              layout="row"
              label={t("settings.runtime.imageEmbeddingDimensions")}
              min={1}
              value={image.dimensions}
              onChange={(value) => updateImageEmbeddingDraft({ dimensions: value })}
            />
            <NumberField
              id="runtime-image-embedding-timeout"
              layout="row"
              label={t("settings.runtime.imageEmbeddingTimeout")}
              min={1}
              value={image.timeout_secs}
              onChange={(value) => updateImageEmbeddingDraft({ timeout_secs: value })}
            />
            <p className="text-xs text-muted-foreground" data-testid="runtime-image-embedding-test-scope">
              {t("settings.runtime.imageEmbeddingTestScope")}
            </p>
          </>
        ) : null}
      </div>
    </SettingsBlock>
  );
}
