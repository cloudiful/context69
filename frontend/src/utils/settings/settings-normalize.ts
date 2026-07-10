import type {
  DoclingSettingsResponse,
  RuntimeSettingsResponse,
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
} from "../../services/api";
import type {
  DoclingPayloadShape,
  DraftDoclingSettings,
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
      base_url: value.embedding.base_url.trim(),
      model: value.embedding.model.trim(),
      dimensions: Number(value.embedding.dimensions),
      timeout_secs: Number(value.embedding.timeout_secs),
      api_key: cleanOptional("api_key" in value.embedding ? value.embedding.api_key : undefined),
      clear_api_key: "clear_api_key" in value.embedding && !!value.embedding.clear_api_key,
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
  value: DraftDoclingSettings | DoclingPayloadShape,
): UpdateDoclingSettingsRequest {
  const payload = value;
  const vlm = payload.vlm ?? {};

  return {
    connection: {
      base_url: payload.connection.base_url.trim(),
      timeout_secs: Number(payload.connection.timeout_secs),
      poll_interval_secs: Number(payload.connection.poll_interval_secs),
    },
    vlm: {
      openai_base_url: cleanOptional(vlm.openai_base_url),
      api_key: cleanOptional(vlm.api_key),
      clear_api_key: !!vlm.clear_api_key,
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
