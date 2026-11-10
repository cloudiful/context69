import { useTranslation } from "react-i18next";

import { SettingsSection } from "@/components/common/settings-section";
import { SettingsGate } from "@/features/settings/settings-gate";

import { FileLibraryBlock } from "./file-library-block";

export function FileLibrarySection() {
  const { t } = useTranslation();

  return (
    <SettingsSection title={t("settings.sections.fileLibrary")}>
      <SettingsGate>
        <FileLibraryBlock />
      </SettingsGate>
    </SettingsSection>
  );
}
