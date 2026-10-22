import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { SettingsBlock } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { ToggleField } from "@/components/common/toggle-field";
import { useSettings } from "@/features/settings/settings-context";

import { TestConnectionButton } from "./test-connection-button";

export function FileLibraryBlock() {
  const { t } = useTranslation();
  const { page } = useSettings();
  const { runtimeDraft, updateRuntimeDraft, s3Testing, testS3Connection } = page;
  const library = runtimeDraft.file_library;

  return (
    <SettingsBlock
      id="settings-file-library"
      compact
      title={t("settings.runtime.fileLibraryTitle")}
      actions={
        library.s3_enabled ? (
          <TestConnectionButton
            testing={s3Testing}
            testId="runtime-file-library-s3-test"
            label={t("settings.runtime.s3Test")}
            onTest={() => void testS3Connection()}
          />
        ) : null
      }
    >
      <div className="grid max-w-2xl gap-3">
        <TextField
          id="runtime-file-library-root"
          label={t("settings.runtime.fileLibraryRoot")}
          value={library.storage_root}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.storage_root = value;
          })}
        />
        <NumberField
          id="runtime-file-library-max-upload"
          label={t("settings.runtime.fileLibraryMaxUploadSize")}
          min={1}
          value={library.max_upload_size_mb}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.max_upload_size_mb = value;
          })}
        />
        <NumberField
          id="runtime-file-library-max-request"
          label={t("settings.runtime.fileLibraryMaxRequestSize")}
          min={1}
          value={library.max_upload_request_size_mb}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.max_upload_request_size_mb = value;
          })}
        />
        <NumberField
          id="runtime-file-library-concurrency"
          label={t("settings.runtime.fileLibraryConcurrency")}
          min={1}
          value={library.ingest_concurrency}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.ingest_concurrency = value;
          })}
        />
        <NumberField
          id="runtime-file-library-url-import-concurrency"
          label={t("settings.runtime.fileLibraryUrlImportConcurrency")}
          min={1}
          value={library.url_import_concurrency}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.url_import_concurrency = value;
          })}
        />
        <NumberField
          id="runtime-file-library-url-import-min-interval"
          label={t("settings.runtime.fileLibraryUrlImportMinInterval")}
          min={1}
          value={library.url_import_min_interval_ms}
          onChange={(value) => updateRuntimeDraft((draft) => {
            draft.file_library.url_import_min_interval_ms = value;
          })}
        />
      </div>

      <div className="grid max-w-2xl gap-3">
        <ToggleField
          id="runtime-file-library-trusted-proxy"
          testId="runtime-file-library-trusted-proxy"
          label={t("settings.runtime.trustedProxyEnabled")}
          helper={t("settings.runtime.trustedProxyHelper")}
          checked={library.trusted_proxy_enabled}
          onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
            draft.file_library.trusted_proxy_enabled = checked;
          })}
        />
        <ToggleField
          id="runtime-file-library-s3-enabled"
          testId="runtime-file-library-s3-enabled"
          label={t("settings.runtime.s3Enabled")}
          checked={library.s3_enabled}
          onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
            draft.file_library.s3_enabled = checked;
          })}
        />
      </div>

      {library.s3_enabled ? (
        <div className="grid max-w-2xl gap-3">
          <TextField
            id="runtime-file-library-s3-endpoint"
            type="url"
            label={t("settings.runtime.s3Endpoint")}
            value={library.s3.endpoint}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.endpoint = value;
            })}
          />
          <TextField
            id="runtime-file-library-s3-region"
            label={t("settings.runtime.s3Region")}
            value={library.s3.region}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.region = value;
            })}
          />
          <TextField
            id="runtime-file-library-s3-bucket"
            label={t("settings.runtime.s3Bucket")}
            value={library.s3.bucket}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.bucket = value;
            })}
          />
          <TextField
            id="runtime-file-library-s3-prefix"
            label={t("settings.runtime.s3Prefix")}
            value={library.s3.prefix}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.prefix = value;
            })}
          />
          <TextField
            id="runtime-file-library-s3-access-key"
            label={t("settings.runtime.s3AccessKey")}
            value={library.s3.access_key}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.access_key = value;
            })}
          />
          <TextField
            id="runtime-file-library-s3-secret-key"
            type="password"
            autoComplete="new-password"
            label={t("settings.runtime.s3SecretKey")}
            value={library.s3.secret_key}
            onChange={(value) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.secret_key = value;
            })}
          />
          <ToggleField
            id="runtime-file-library-s3-path-style"
            label={t("settings.runtime.s3PathStyle")}
            checked={library.s3.path_style}
            onCheckedChange={(checked) => updateRuntimeDraft((draft) => {
              draft.file_library.s3.path_style = checked;
            })}
          />
        </div>
      ) : null}
    </SettingsBlock>
  );
}
