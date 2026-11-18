import { useTranslation } from "react-i18next";

import { FormField, type FieldLayout } from "@/components/common/form-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Input } from "@/components/ui/input";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

/**
 * A numeric field whose blank state is an inheritance.
 *
 * The shared number field turns an empty input into a zero, but a zero here is
 * a value: the backend refuses it instead of reading it as an inheritance. This
 * adapter keeps the blank state as `null`, so an empty field means "inherit
 * the document value" and never reaches the wire as a zero.
 */
export function OptionalNumberField({
  id,
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  helper,
  placeholder,
  disabled,
  testId,
  layout,
}: {
  id: string;
  label: string;
  value: number | null;
  onChange: (value: number | null) => void;
  min?: number;
  max?: number;
  step?: number;
  helper?: string;
  placeholder?: string;
  disabled?: boolean;
  testId?: string;
  layout?: FieldLayout;
}) {
  return (
    <FormField
      label={label}
      htmlFor={id}
      helper={helper}
      layout={layout}
      controlClassName="max-w-28"
    >
      <Input
        id={id}
        name={id}
        type="number"
        inputMode="decimal"
        value={value === null ? "" : String(value)}
        placeholder={placeholder}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        data-testid={testId}
        onChange={(event) => {
          const raw = event.target.value;
          if (raw.trim() === "") {
            onChange(null);
            return;
          }
          const parsed = Number(raw);
          // An unparsable entry is not a value: it stays an inheritance
          // instead of becoming a zero.
          onChange(Number.isFinite(parsed) ? parsed : null);
        }}
      />
    </FormField>
  );
}

/**
 * The optional Git code embedding model block.
 *
 * Every provider field is an override: a blank one inherits the document
 * configuration field by field, and the block shows the inherited value as
 * the placeholder. Details are only edited while enabled, but the draft keeps
 * them when the block is turned off so a later save can re-enable the same
 * configuration.
 */
export function CodeEmbeddingBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateCodeEmbeddingDraft } = page;
  const code = runtimeDraft.code_embedding;
  const document = runtimeDraft.embedding;
  // The effective code model is the document model while no code model is
  // stored, so a document model change moves the code identity with it.
  const inheritsModel = !code.model.trim();
  // The blank query field resolves to the document query model, then to the
  // effective code model — the same order the save and the probe use.
  const effectiveModel = code.model.trim() || document.model.trim();
  const inheritedQueryModel = document.query_model.trim() || effectiveModel;
  // Inherited dimensions move the vector identity with the document too.
  const inheritsDimensions = code.dimensions === null;

  return (
    <SettingsBlock
      id="settings-code-embedding"
      compact
      title={t("settings.runtime.codeEmbeddingTitle")}
      actions={
        <TestConnectionButton
          testing={page.codeEmbeddingTesting}
          disabled={!code.enabled}
          testId="runtime-code-embedding-test"
          label={t("settings.runtime.codeEmbeddingTest")}
          onTest={() => void page.testCodeEmbeddingConnection()}
        />
      }
    >
      <div className="grid max-w-3xl gap-3">
        <ToggleField
          id="runtime-code-embedding-enabled"
          testId="runtime-code-embedding-enabled"
          layout="row"
          label={t("settings.runtime.codeEmbeddingEnabled")}
          helper={t("settings.runtime.codeEmbeddingEnabledHelper")}
          checked={code.enabled}
          onCheckedChange={(checked) => updateCodeEmbeddingDraft({ enabled: checked })}
        />
        {code.enabled ? (
          <>
            <TextField
              id="runtime-code-embedding-base-url"
              type="url"
              layout="row"
              controlClassName="max-w-xl"
              label={t("settings.runtime.codeEmbeddingBaseUrl")}
              helper={t("settings.runtime.codeEmbeddingBaseUrlHelper")}
              placeholder={document.base_url.trim() || "https://openrouter.ai/api/v1"}
              value={code.base_url}
              onChange={(value) => updateCodeEmbeddingDraft({ base_url: value })}
            />
            <TextField
              id="runtime-code-embedding-api-key"
              type="password"
              autoComplete="new-password"
              layout="row"
              label={t("settings.runtime.codeEmbeddingApiKey")}
              helper={t("settings.runtime.codeEmbeddingApiKeyHelper")}
              placeholder={code.has_api_key ? t("settings.runtime.codeEmbeddingKeyStored") : "sk-..."}
              value={code.api_key}
              onChange={(value) => updateCodeEmbeddingDraft({ api_key: value })}
            />
            {code.has_api_key ? (
              <ToggleField
                id="runtime-code-embedding-clear-key"
                testId="runtime-code-embedding-clear-key"
                layout="row"
                label={t("settings.runtime.codeEmbeddingClearKey")}
                helper={t("settings.runtime.codeEmbeddingClearKeyHelper")}
                checked={code.clear_api_key}
                onCheckedChange={(checked) => updateCodeEmbeddingDraft({ clear_api_key: checked })}
              />
            ) : null}
            {code.inherits_document_api_key ? (
              <p className="text-xs text-muted-foreground" data-testid="runtime-code-embedding-inherited-key">
                {t("settings.runtime.codeEmbeddingInheritedCredential")}
              </p>
            ) : null}
            <TextField
              id="runtime-code-embedding-model"
              layout="row"
              label={t("settings.runtime.codeEmbeddingModel")}
              helper={t("settings.runtime.codeEmbeddingModelHelper")}
              placeholder={document.model.trim() || undefined}
              value={code.model}
              onChange={(value) => updateCodeEmbeddingDraft({ model: value })}
            />
            <TextField
              id="runtime-code-embedding-query-model"
              layout="row"
              label={t("settings.runtime.codeEmbeddingQueryModel")}
              helper={t("settings.runtime.codeEmbeddingQueryModelHelper")}
              placeholder={inheritedQueryModel || undefined}
              value={code.query_model}
              onChange={(value) => updateCodeEmbeddingDraft({ query_model: value })}
            />
            <OptionalNumberField
              id="runtime-code-embedding-dimensions"
              layout="row"
              label={t("settings.runtime.codeEmbeddingDimensions")}
              helper={t("settings.runtime.codeEmbeddingDimensionsHelper")}
              min={1}
              placeholder={String(document.dimensions)}
              value={code.dimensions}
              onChange={(value) => updateCodeEmbeddingDraft({ dimensions: value })}
            />
            <OptionalNumberField
              id="runtime-code-embedding-timeout"
              layout="row"
              label={t("settings.runtime.codeEmbeddingTimeout")}
              helper={t("settings.runtime.codeEmbeddingTimeoutHelper")}
              min={1}
              placeholder={String(document.timeout_secs)}
              value={code.timeout_secs}
              onChange={(value) => updateCodeEmbeddingDraft({ timeout_secs: value })}
            />
            {inheritsModel || inheritsDimensions ? (
              <p className="text-xs text-muted-foreground" data-testid="runtime-code-embedding-inherited-identity">
                {t("settings.runtime.codeEmbeddingInheritedIdentityWarning")}
              </p>
            ) : null}
          </>
        ) : null}
      </div>
    </SettingsBlock>
  );
}
