import { useTranslation } from "react-i18next";

import { SettingsSection } from "@/components/common/settings-section";
import { SettingsGate } from "@/features/settings/settings-gate";

import { EmbeddingBlock } from "./embedding-controls";
import { CodeEmbeddingBlock } from "./code-embedding-controls";

export function EmbeddingSection() {
  const { t } = useTranslation();

  return (
    <SettingsSection title={t("settings.sections.embedding")}>
      <SettingsGate>
        <div className="grid gap-3">
          <EmbeddingBlock />
          <CodeEmbeddingBlock />
        </div>
      </SettingsGate>
    </SettingsSection>
  );
}
