import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { FormField } from "@/components/common/form-field";
import { NumberField } from "@/components/common/number-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
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

  return (
    <SettingsSection title={t("settings.runtime.title")}>
      <SettingsGate>
      <div className="grid gap-3">
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
            {page.requiresRebuild ? (
              <div
                role="status"
                data-testid="runtime-requires-rebuild"
                className="grid gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-3"
              >
                <div className="flex items-start gap-2 text-sm">
                  <AlertTriangle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
                  <div className="grid gap-1">
                    <p>{t("settings.runtime.requiresRebuild")}</p>
                    <p className="text-muted-foreground">
                      {t("settings.runtime.requiresRebuildDimensions")}
                    </p>
                  </div>
                </div>
                <div>
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
              </div>
            ) : null}
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
              helper={t("settings.runtime.chunkingUnitHelper")}
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
                <FormField
                  label={t("settings.runtime.chunkingTokenizer")}
                  htmlFor="runtime-chunking-tokenizer"
                  helper={t("settings.runtime.chunkingTokenizerHelper")}
                >
                  <Select
                    value={runtimeDraft.chunking.tokenizer}
                    onValueChange={(value) => updateRuntimeDraft((draft) => {
                      draft.chunking.tokenizer = value as DraftChunkingTokenizer;
                    })}
                  >
                    <SelectTrigger
                      id="runtime-chunking-tokenizer"
                      aria-label={t("settings.runtime.chunkingTokenizer")}
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
                {runtimeDraft.chunking.tokenizer === "hf_local" ? (
                  tokenizerArtifacts.length > 0 ? (
                    <FormField
                      label={t("settings.runtime.chunkingTokenizerArtifact")}
                      htmlFor="runtime-chunking-artifact"
                      helper={t("settings.runtime.chunkingTokenizerArtifactHelper")}
                    >
                      <Select
                        value={runtimeDraft.chunking.tokenizer_sha256 ?? undefined}
                        onValueChange={(value) => updateRuntimeDraft((draft) => {
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
                  ) : (
                    <p
                      data-testid="runtime-chunking-artifact-empty"
                      className="text-xs text-muted-foreground"
                    >
                      {t("settings.runtime.chunkingTokenizerArtifactEmpty")}
                    </p>
                  )
                ) : null}
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
            <p className="text-xs text-muted-foreground">{t("settings.runtime.chunkingHelper")}</p>
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
