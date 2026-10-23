import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { NumberField } from "@/components/common/number-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";

import { FileLibraryBlock } from "./file-library-block";
import { TestConnectionButton, VectorRebuildControl } from "./test-connection-button";

export function RuntimeSection() {
  const { t } = useTranslation();
  const { page, vector } = useSettings();
  const { runtimeDraft, updateRuntimeDraft, updateQdrantUrl } = page;

  return (
    <SettingsSection title={t("settings.runtime.title")}>
      <SettingsGate>
      <div className="grid gap-3">
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
              label={t("settings.runtime.embeddingModel")}
              value={runtimeDraft.embedding.model}
              onChange={(value) => updateRuntimeDraft((draft) => {
                draft.embedding.model = value;
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
            <ToggleField
              id="runtime-qdrant-recreate"
              testId="runtime-qdrant-recreate"
              label={t("settings.runtime.recreateOnDimensionMismatch")}
              checked={runtimeDraft.qdrant.recreate_on_dimension_mismatch}
              onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
                draft.qdrant.recreate_on_dimension_mismatch = checked;
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
