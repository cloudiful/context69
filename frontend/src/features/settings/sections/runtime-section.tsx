import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { FormField } from "@/components/common/form-field";
import { NumberField } from "@/components/common/number-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import type { DraftChunkingTokenizer, DraftChunkingUnit } from "@/utils/settings";

import { EmbeddingBlock } from "./embedding-controls";
import { FileLibraryBlock } from "./file-library-block";
import { TestConnectionButton, VectorRebuildControl } from "./test-connection-button";

export function RuntimeSection() {
  const { t } = useTranslation();
  const { page, vector } = useSettings();
  const { runtimeDraft, updateRuntimeDraft, updateQdrantUrl } = page;
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
  const inputLimitEnabled = runtimeDraft.embedding.input_policy.max_input_tokens != null;

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
      draft.embedding.input_policy.add_special_tokens = false;
    });
  }

  function retryOrPreparePreset() {
    if (selectedPreset) {
      void page.prepareTokenizerPreset(selectedPreset);
    }
  }

  return (
    <SettingsSection title={t("settings.runtime.title")}>
      <SettingsGate>
      <div className="grid gap-3">
        {page.requiresRebuild ? (
          <Alert
            role="status"
            data-testid="runtime-requires-rebuild"
            className="border-amber-500/40 bg-amber-500/5"
          >
            <AlertTriangle
              aria-hidden="true"
              className="text-amber-600 dark:text-amber-400"
            />
            <AlertDescription>{t("settings.runtime.requiresRebuild")}</AlertDescription>
            <div className="col-span-full">
              <Button
                type="button"
                variant="outline"
                size="sm"
                data-testid="runtime-requires-rebuild-action"
                disabled={vector.running}
                onClick={() => vector.setConfirmOpen(true)}
              >
                {t("settings.runtime.vectorRebuild")}
              </Button>
            </div>
          </Alert>
        ) : null}

        <EmbeddingBlock />

        <SettingsBlock
          id="settings-qdrant"
          compact
          title={t("settings.runtime.qdrantTitle")}
          actions={
            <TestConnectionButton
              testing={page.qdrantTesting}
              disabled={!runtimeDraft.qdrant.url.trim()}
              testId="runtime-qdrant-test"
              label={t("settings.runtime.qdrantTest")}
              onTest={() => void page.testQdrantConnection()}
            />
          }
        >
          <div className="grid max-w-2xl gap-3">
            <TextField
              id="runtime-qdrant-url"
              type="url"
              label={t("settings.runtime.qdrantUrl")}
              value={runtimeDraft.qdrant.url}
              onChange={updateQdrantUrl}
            />
            <TextField
              id="runtime-qdrant-collection"
              label={t("settings.runtime.qdrantCollection")}
              value={runtimeDraft.qdrant.collection_name}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.qdrant.collection_name = value;
              })}
            />
            <VectorRebuildControl />
          </div>
        </SettingsBlock>

        <SettingsBlock
          id="settings-scheduler"
          compact
          title={t("settings.runtime.schedulerTitle")}
          actions={
            <TestConnectionButton
              testing={page.valkeyTesting}
              disabled={!runtimeDraft.scheduler.valkey_url.trim()}
              testId="runtime-valkey-test"
              label={t("settings.runtime.valkeyTest")}
              onTest={() => void page.testValkeyConnection()}
            />
          }
        >
          <div className="grid max-w-2xl gap-3">
            <NumberField
              id="runtime-scheduler-interval"
              label={t("settings.runtime.schedulerInterval")}
              min={1}
              value={runtimeDraft.scheduler.interval_secs}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.interval_secs = value;
              })}
            />
            <NumberField
              id="runtime-scheduler-concurrency"
              label={t("settings.runtime.schedulerConcurrency")}
              min={1}
              value={runtimeDraft.scheduler.max_concurrency}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.max_concurrency = value;
              })}
            />
            <TextField
              id="runtime-scheduler-job-id"
              label={t("settings.runtime.schedulerJobId")}
              value={runtimeDraft.scheduler.job_id}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.job_id = value;
              })}
            />
            <TextField
              id="runtime-scheduler-valkey-url"
              label={t("settings.runtime.schedulerValkeyUrl")}
              helper={t("settings.runtime.valkeyRestartRequired")}
              placeholder="redis://valkey:6379/0"
              value={runtimeDraft.scheduler.valkey_url}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.scheduler.valkey_url = value;
              })}
            />
            <ToggleField
              id="runtime-scheduler-run-on-start"
              testId="runtime-scheduler-run-on-start"
              label={t("settings.runtime.runOnStart")}
              checked={runtimeDraft.scheduler.run_on_start}
              onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
                draft.scheduler.run_on_start = checked;
              })}
            />
          </div>
        </SettingsBlock>

        <SettingsBlock id="settings-chunking" compact title={t("settings.runtime.chunkingTitle")}>
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

            {/*
              The one editable tokenizer. It measures the token budget and
              governs the input check, so the two never disagree.
            */}
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
              A deployment that stored an input envelope of its own is told
              once that the next save aligns it with the lower selection.
            */}
            {!page.inputPolicyAligned && inputLimitEnabled ? (
              <Alert
                role="status"
                data-testid="runtime-chunking-input-policy-warning"
                className="border-amber-500/40 bg-amber-500/5 text-xs"
              >
                <AlertDescription>
                  {t("settings.runtime.chunkingTokenizerInputPolicyWarning")}
                </AlertDescription>
              </Alert>
            ) : null}

            {/*
              One flat first-preparation row: the non-commercial link and the
              explicit action sit inline, and a ready preset renders nothing.
            */}
            {selectedPreset && selectedPresetStatus !== "ready" ? (
              <div
                data-testid={`runtime-tokenizer-preset-${selectedPreset.id}`}
                className="flex flex-wrap items-center gap-3 text-xs"
              >
                <a
                  className="underline"
                  href={selectedPreset.license_url}
                  title={selectedPreset.license}
                  target="_blank"
                  rel="noreferrer"
                >
                  {t("settings.runtime.chunkingTokenizerPresetLicense")}
                </a>
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

        <FileLibraryBlock />
      </div>
      </SettingsGate>

      <ConfirmDialog
        open={vector.confirmOpen}
        title={t("settings.runtime.vectorRebuild")}
        message={t("settings.runtime.vectorRebuildConfirm")}
        acceptLabel={t("settings.runtime.vectorRebuild")}
        onAccept={() => {
          vector.setConfirmOpen(false);
          void vector.start();
        }}
        onCancel={() => vector.setConfirmOpen(false)}
      />
    </SettingsSection>
  );
}
