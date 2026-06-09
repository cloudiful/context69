import type {
  DoclingSettingsResponse,
  RuntimeSettingsResponse,
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
} from "../../services/api";
import type {
  DoclingPayloadShape,
  DraftDoclingSettingsEnvelope,
  DraftRuntimeSettings,
} from "./settings-types";
import { doclingResponseToDraft, runtimeResponseToDraft } from "./settings-drafts";

export function runtimeResponseToPayload(
  response: RuntimeSettingsResponse,
): UpdateRuntimeSettingsRequest {
  return normalizeRuntimePayload(runtimeResponseToDraft(response));
}

export function normalizeRuntimePayload(
  value: DraftRuntimeSettings | UpdateRuntimeSettingsRequest,
): UpdateRuntimeSettingsRequest {
  return {
    qdrant: {
      url: value.qdrant.url.trim(),
      collection_name: value.qdrant.collection_name.trim(),
      recreate_on_dimension_mismatch: value.qdrant.recreate_on_dimension_mismatch,
    },
    embedding: {
      provider_account_key: value.embedding.provider_account_key.trim(),
      model: value.embedding.model.trim(),
      dimensions: Number(value.embedding.dimensions),
      timeout_secs: Number(value.embedding.timeout_secs),
    },
    scheduler: {
      interval_secs: Number(value.scheduler.interval_secs),
      run_on_start: value.scheduler.run_on_start,
      max_concurrency: Number(value.scheduler.max_concurrency),
      job_id: value.scheduler.job_id.trim(),
      valkey_url: cleanOptional(value.scheduler.valkey_url ?? ""),
    },
    chunking: {
      max_chars: Number(value.chunking.max_chars),
      overlap_chars: Number(value.chunking.overlap_chars),
    },
    file_library: {
      storage_root: value.file_library.storage_root.trim(),
      max_upload_size_mb: Number(value.file_library.max_upload_size_mb),
      max_upload_request_size_mb: Number(value.file_library.max_upload_request_size_mb),
      ingest_concurrency: Number(value.file_library.ingest_concurrency),
      pdf_pages_per_task: Number(value.file_library.pdf_pages_per_task),
    },
  };
}

export function doclingResponseToPayload(
  response: DoclingSettingsResponse,
): UpdateDoclingSettingsRequest {
  return normalizeDoclingPayload(doclingResponseToDraft(response));
}

export function normalizeDoclingPayload(
  value: DraftDoclingSettingsEnvelope | DoclingPayloadShape,
): UpdateDoclingSettingsRequest {
  const payload = "draft" in value
    ? {
        ...value.draft,
        ocr: {
          ...value.draft.ocr,
          ocr_lang_text: value.ocrLangText,
        },
      }
    : value;
  const ocrLangTextValue = payload.ocr.ocr_lang_text ?? "";
  const conversion = payload.conversion ?? {};
  const enrichment = payload.enrichment ?? {};
  const vlm = payload.vlm ?? {};

  return {
    connection: {
      base_url: payload.connection.base_url.trim(),
      timeout_secs: Number(payload.connection.timeout_secs),
      poll_interval_secs: Number(payload.connection.poll_interval_secs),
    },
    conversion: {
      pdf_backend: cleanOptional(conversion.pdf_backend),
      images_scale: normalizeOptionalNumber(conversion.images_scale),
      image_export_mode: cleanOptional(conversion.image_export_mode),
    },
    ocr: {
      do_ocr: payload.ocr.do_ocr,
      force_ocr: payload.ocr.force_ocr,
      ocr_engine: cleanOptional(payload.ocr.ocr_engine),
      ocr_lang: parseOcrLang(ocrLangTextValue),
    },
    enrichment: {
      do_code_enrichment: !!enrichment.do_code_enrichment,
      do_formula_enrichment: !!enrichment.do_formula_enrichment,
      do_picture_description: !!enrichment.do_picture_description,
    },
    vlm: {
      provider_account_key: cleanOptional(vlm.provider_account_key),
      vlm_pipeline_model: cleanOptional(vlm.vlm_pipeline_model),
      picture_description_model: cleanOptional(vlm.picture_description_model),
      code_formula_model: cleanOptional(vlm.code_formula_model),
    },
  };
}

function cleanOptional(value: string | null | undefined) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function normalizeOptionalNumber(value: number | null | undefined) {
  return typeof value === "number" && !Number.isNaN(value) ? value : undefined;
}

function parseOcrLang(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}
