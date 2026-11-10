import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { NumberField } from "@/components/common/number-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import type { DraftChunkingTokenizer, DraftChunkingUnit } from "@/utils/settings";

export function ChunkingSection() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft } = page;
  const tokenizerArtifacts = page.runtimeQuery.data?.tokenizer_artifacts ?? [];
  const tokenizerPresets = page.runtimeQuery.data?.tokenizer_presets ?? [];

  const chunking = runtimeDraft.chunking;
  // The owner derives the selected preset; this section never repeats the
  // lookup, so the control, the status and the Save guard agree.
  const selectedPreset = page.selectedTokenizerPreset;
  const selectedPresetStatus = page.selectedTokenizerStatus;
  const customSelection = chunking.tokenizer === "hf_local"
    && !selectedPreset
    && tokenizerArtifacts.length > 0;
  // A stored custom hash that is no longer mounted stays diagnosable instead
  // of silently rendering an empty control.
  const missingCustomSelection = chunking.tokenizer === "hf_local"
    && !selectedPreset
    && chunking.tokenizer_sha256 != null
    && !tokenizerArtifacts.some((artifact) => artifact.sha256 === chunking.tokenizer_sha256);
  const tokenizerValue = selectedPreset
    ? `preset:${selectedPreset.id}`
    : customSelection
      ? "custom"
      : chunking.tokenizer;
  const showAdvancedArtifact = tokenizerArtifacts.length > 0 || missingCustomSelection;

  function selectTokenizer(value: string) {
    const preset = tokenizerPresets.find((candidate) => `preset:${candidate.id}` === value);
    updateRuntimeDraft((draft) => {
      if (preset) {
        draft.chunking.tokenizer = "hf_local";
        draft.chunking.tokenizer_sha256 = preset.sha256;
        return;
      }
      if (value === "custom") {
        draft.chunking.tokenizer = "hf_local";
        // A custom selection always names one mounted artifact: an existing
        // mounted hash is kept, and a built-in selection moves to the first
        // one. Without a mounted artifact the option is not offered at all.
        const current = draft.chunking.tokenizer_sha256;
        const mounted = tokenizerArtifacts.some((artifact) => artifact.sha256 === current);
        if (!mounted) {
          draft.chunking.tokenizer_sha256 = tokenizerArtifacts[0]?.sha256 ?? null;
        }
        return;
      }
      draft.chunking.tokenizer = value as DraftChunkingTokenizer;
      // A built-in tokenizer counts without special tokens and pins no file.
      draft.chunking.tokenizer_sha256 = null;
    });
  }

  function retryOrPreparePreset() {
    if (selectedPreset) {
      void page.prepareTokenizerPreset(selectedPreset);
    }
  }

  return (
    <SettingsSection title={t("settings.sections.chunking")}>
      <SettingsGate>
        <SettingsBlock
          id="settings-chunking"
          compact
          title={t("settings.runtime.chunkingTitle")}
        >
          <div className="grid max-w-2xl gap-3">
            <FormField
              label={t("settings.runtime.chunkingUnit")}
              htmlFor="runtime-chunking-unit"
            >
              <Select
                value={runtimeDraft.chunking.unit}
                onValueChange={(value) => updateRuntimeDraft((draft) => {
                  draft.chunking.unit = value as DraftChunkingUnit;
                })}
              >
                <SelectTrigger
                  id="runtime-chunking-unit"
                  aria-label={t("settings.runtime.chunkingUnit")}
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="chars">{t("settings.runtime.chunkingUnitChars")}</SelectItem>
                  <SelectItem value="tokens">{t("settings.runtime.chunkingUnitTokens")}</SelectItem>
                </SelectContent>
              </Select>
            </FormField>

            <FormField
              label={t("settings.runtime.chunkingTokenizer")}
              htmlFor="runtime-chunking-tokenizer"
            >
              <Select value={tokenizerValue} onValueChange={selectTokenizer}>
                <SelectTrigger
                  id="runtime-chunking-tokenizer"
                  aria-label={t("settings.runtime.chunkingTokenizer")}
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="cl100k_base">cl100k_base</SelectItem>
                  <SelectItem value="o200k_base">o200k_base</SelectItem>
                  {tokenizerPresets.map((preset) => (
                    <SelectItem key={preset.id} value={`preset:${preset.id}`}>
                      {preset.label}
                    </SelectItem>
                  ))}
                  {tokenizerArtifacts.length > 0 ? (
                    <SelectItem value="custom">
                      {t("settings.runtime.chunkingTokenizerCustom")}
                    </SelectItem>
                  ) : null}
                </SelectContent>
              </Select>
            </FormField>

            {/*
              One flat first-preparation row: the explicit action and any
              status sit inline, and a ready preset renders nothing.
            */}
            {selectedPreset && selectedPresetStatus !== "ready" ? (
              <div
                data-testid={`runtime-tokenizer-preset-${selectedPreset.id}`}
                className="flex flex-wrap items-center gap-3 text-xs"
              >
                {selectedPresetStatus === "pending" ? (
                  <span
                    data-testid={`runtime-tokenizer-preset-status-${selectedPreset.id}`}
                    className="text-muted-foreground"
                  >
                    {t("settings.runtime.chunkingTokenizerPresetPending")}
                  </span>
                ) : (
                  <>
                    {selectedPresetStatus === "error" ? (
                      <span
                        data-testid={`runtime-tokenizer-preset-status-${selectedPreset.id}`}
                        className="text-destructive"
                      >
                        {`${t("settings.runtime.chunkingTokenizerPresetFailed")}${
                          page.tokenizerPreparationFor(selectedPreset.id)?.message
                            ? ` ${page.tokenizerPreparationFor(selectedPreset.id)?.message}`
                            : ""
                        }`}
                      </span>
                    ) : null}
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      data-testid={`runtime-tokenizer-preset-prepare-${selectedPreset.id}`}
                      onClick={retryOrPreparePreset}
                    >
                      {selectedPresetStatus === "error"
                        ? t("settings.runtime.chunkingTokenizerPresetRetry")
                        : t("settings.runtime.chunkingTokenizerPresetPrepare")}
                    </Button>
                  </>
                )}
              </div>
            ) : null}

            {/*
              The manual path stays available only when it can show something:
              a mounted artifact, or a selected hash that is no longer mounted.
            */}
            {showAdvancedArtifact ? (
              <details className="rounded-md border border-border/60 p-2">
                <summary className="cursor-pointer text-sm">
                  {t("settings.runtime.chunkingTokenizerAdvanced")}
                </summary>
                <div className="mt-3 grid gap-3">
                  {tokenizerArtifacts.length > 0 ? (
                    <FormField
                      label={t("settings.runtime.chunkingTokenizerArtifact")}
                      htmlFor="runtime-chunking-artifact"
                      helper={t("settings.runtime.chunkingTokenizerArtifactHelper")}
                    >
                      <Select
                        value={customSelection ? (chunking.tokenizer_sha256 ?? undefined) : undefined}
                        onValueChange={(value) => updateRuntimeDraft((draft) => {
                          draft.chunking.tokenizer = "hf_local";
                          draft.chunking.tokenizer_sha256 = value;
                        })}
                      >
                        <SelectTrigger
                          id="runtime-chunking-artifact"
                          aria-label={t("settings.runtime.chunkingTokenizerArtifact")}
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
                  ) : null}
                  {missingCustomSelection ? (
                    <p
                      data-testid="runtime-chunking-artifact-missing"
                      className="text-xs text-muted-foreground"
                    >
                      {t("settings.runtime.chunkingTokenizerArtifactMissing", {
                        hash: chunking.tokenizer_sha256?.slice(0, 12) ?? "",
                      })}
                    </p>
                  ) : null}
                </div>
              </details>
            ) : null}

            {runtimeDraft.chunking.unit === "chars" ? (
              <>
                <NumberField
                  id="runtime-chunking-max-chars"
                  label={t("settings.runtime.chunkingMaxChars")}
                  min={1}
                  value={runtimeDraft.chunking.max_chars}
                  onChange={(value) => updateRuntimeDraft((draft) => {
                    draft.chunking.max_chars = value;
                  })}
                />
                <NumberField
                  id="runtime-chunking-overlap"
                  label={t("settings.runtime.chunkingOverlapChars")}
                  min={0}
                  value={runtimeDraft.chunking.overlap_chars}
                  onChange={(value) => updateRuntimeDraft((draft) => {
                    draft.chunking.overlap_chars = value;
                  })}
                />
              </>
            ) : (
              <>
                <NumberField
                  id="runtime-chunking-max-tokens"
                  label={t("settings.runtime.chunkingMaxTokens")}
                  min={1}
                  value={runtimeDraft.chunking.max_tokens}
                  onChange={(value) => updateRuntimeDraft((draft) => {
                    draft.chunking.max_tokens = value;
                  })}
                />
                <NumberField
                  id="runtime-chunking-overlap-tokens"
                  label={t("settings.runtime.chunkingOverlapTokens")}
                  min={0}
                  value={runtimeDraft.chunking.overlap_tokens}
                  onChange={(value) => updateRuntimeDraft((draft) => {
                    draft.chunking.overlap_tokens = value;
                  })}
                />
              </>
            )}
          </div>
        </SettingsBlock>
      </SettingsGate>
    </SettingsSection>
  );
}
