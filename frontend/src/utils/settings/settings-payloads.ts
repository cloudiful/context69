import type {
  DraftDoclingSettings,
  DraftRuntimeSettings,
  DraftSearchSettings,
  ProviderAccountDraft,
} from "./settings-types";
import type {
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
  UpdateSearchSettingsRequest,
  UpsertProviderAccountRequest,
} from "../../services/api";
import { normalizeDoclingPayload, normalizeRuntimePayload } from "./settings-normalize";
import { buildProviderAccountComparablePayload } from "./settings-compare";

export function buildProviderAccountPayload(
  draft: ProviderAccountDraft,
): UpsertProviderAccountRequest {
  const payload = buildProviderAccountComparablePayload(draft);
  const apiKey = draft.api_key.trim();
  if (apiKey) {
    payload.api_key = apiKey;
  }
  return payload;
}

export function buildRuntimePayload(
  draft: DraftRuntimeSettings,
): UpdateRuntimeSettingsRequest {
  return normalizeRuntimePayload({
    qdrant: {
      url: draft.qdrant.url,
      collection_name: draft.qdrant.collection_name,
      recreate_on_dimension_mismatch: draft.qdrant.recreate_on_dimension_mismatch,
    },
    embedding: {
      provider_account_key: draft.embedding.provider_account_key,
      model: draft.embedding.model,
      dimensions: draft.embedding.dimensions,
      timeout_secs: draft.embedding.timeout_secs,
    },
    scheduler: {
      interval_secs: draft.scheduler.interval_secs,
      run_on_start: draft.scheduler.run_on_start,
      max_concurrency: draft.scheduler.max_concurrency,
      job_id: draft.scheduler.job_id,
      valkey_url: draft.scheduler.valkey_url,
    },
    chunking: {
      max_chars: draft.chunking.max_chars,
      overlap_chars: draft.chunking.overlap_chars,
    },
    file_library: {
      storage_root: draft.file_library.storage_root,
      max_upload_size_mb: draft.file_library.max_upload_size_mb,
      max_upload_request_size_mb: draft.file_library.max_upload_request_size_mb,
      ingest_concurrency: draft.file_library.ingest_concurrency,
      pdf_pages_per_task: draft.file_library.pdf_pages_per_task,
    },
  });
}

export function buildDoclingPayload(
  draft: DraftDoclingSettings,
  ocrLangText: string,
): UpdateDoclingSettingsRequest {
  return normalizeDoclingPayload({
    connection: {
      base_url: draft.connection.base_url,
      timeout_secs: draft.connection.timeout_secs,
      poll_interval_secs: draft.connection.poll_interval_secs,
    },
    conversion: {
      pdf_backend: draft.conversion.pdf_backend,
      images_scale: draft.conversion.images_scale,
      image_export_mode: draft.conversion.image_export_mode,
    },
    ocr: {
      do_ocr: draft.ocr.do_ocr,
      force_ocr: draft.ocr.force_ocr,
      ocr_engine: draft.ocr.ocr_engine,
      ocr_lang_text: ocrLangText,
    },
    enrichment: {
      do_code_enrichment: draft.enrichment.do_code_enrichment,
      do_formula_enrichment: draft.enrichment.do_formula_enrichment,
      do_picture_description: draft.enrichment.do_picture_description,
    },
    vlm: {
      provider_account_key: draft.vlm.provider_account_key,
      vlm_pipeline_model: draft.vlm.vlm_pipeline_model,
      picture_description_model: draft.vlm.picture_description_model,
      code_formula_model: draft.vlm.code_formula_model,
    },
  });
}

export function buildSearchSettingsPayload(
  draft: DraftSearchSettings,
  rerankApiKeyDraft: string,
  clearStoredRerankApiKey: boolean,
): UpdateSearchSettingsRequest {
  const payload: UpdateSearchSettingsRequest = {
    ...buildSearchSettingsComparablePayload(draft),
    clear_api_key: clearStoredRerankApiKey,
  };
  const apiKey = rerankApiKeyDraft.trim();
  if (apiKey) {
    payload.api_key = apiKey;
  }
  return payload;
}

export function buildSearchSettingsComparablePayload(
  draft: DraftSearchSettings,
) {
  return {
    mode: draft.mode,
    rerank_enabled: draft.rerank_enabled,
    rerank_base_url: draft.rerank_base_url.trim(),
    rerank_model: draft.rerank_model.trim(),
    candidate_limit: Number(draft.candidate_limit),
    timeout_secs: Number(draft.timeout_secs),
  };
}
