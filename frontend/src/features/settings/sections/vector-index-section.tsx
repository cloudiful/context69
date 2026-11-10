import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton, VectorRebuildControl } from "./test-connection-button";

export function VectorIndexSection() {
  const { t } = useTranslation();
  const { page, vector } = useSettings();
  const { runtimeDraft, updateRuntimeDraft, updateQdrantUrl } = page;

  return (
    <SettingsSection title={t("settings.sections.vectorIndex")}>
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
