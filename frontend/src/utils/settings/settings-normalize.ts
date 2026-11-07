import type {
  DoclingSettingsResponse,
  RuntimeSettingsResponse,
  UpdateDoclingSettingsRequest,
} from "../../services/api";
import type { UpdateRuntimeSettingsRequest } from "../../services/api";
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
  value: DraftRuntimeSettings,
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
      query_model: cleanOptional(value.embedding.query_model),
      dimensions: Number(value.embedding.dimensions),
      timeout_secs: Number(value.embedding.timeout_secs),
      api_key: cleanOptional("api_key" in value.embedding ? value.embedding.api_key : undefined),
      batching: {
        max_items_per_request: Number(value.embedding.batching.max_items_per_request),
        max_item_chars: Number(value.embedding.batching.max_item_chars),
        max_request_chars: Number(value.embedding.batching.max_request_chars),
        max_request_bytes: Number(value.embedding.batching.max_request_bytes),
      },
      input_policy: {
        tokenizer: value.embedding.input_policy.tokenizer,
        tokenizer_sha256: value.embedding.input_policy.tokenizer_sha256 ?? null,
        max_input_tokens:
          value.embedding.input_policy.max_input_tokens == null
            ? null
            : Number(value.embedding.input_policy.max_input_tokens),
        document_server_prefix: value.embedding.input_policy.document_server_prefix,
        query_server_prefix: value.embedding.input_policy.query_server_prefix,
        add_special_tokens: value.embedding.input_policy.add_special_tokens,
      },
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
      unit: value.chunking.unit,
      tokenizer: value.chunking.tokenizer,
      tokenizer_sha256: value.chunking.tokenizer_sha256 ?? null,
      max_tokens: Number(value.chunking.max_tokens),
      overlap_tokens: Number(value.chunking.overlap_tokens),
    },
    file_library: {
      storage_root: value.file_library.storage_root.trim(),
      max_upload_size_mb: Number(value.file_library.max_upload_size_mb),
      max_upload_request_size_mb: Number(value.file_library.max_upload_request_size_mb),
      ingest_concurrency: Number(value.file_library.ingest_concurrency),
      url_import_concurrency: Number(value.file_library.url_import_concurrency),
      url_import_min_interval_ms: Number(value.file_library.url_import_min_interval_ms),
      trusted_proxy_enabled: value.file_library.trusted_proxy_enabled,
      s3: value.file_library.s3_enabled && value.file_library.s3
        ? {
            endpoint: value.file_library.s3.endpoint.trim(),
            region: value.file_library.s3.region.trim(),
            bucket: value.file_library.s3.bucket.trim(),
            prefix: value.file_library.s3.prefix.trim().replace(/^\/+|\/+$/g, ""),
            path_style: value.file_library.s3.path_style,
            access_key: value.file_library.s3.access_key.trim(),
            secret_key: cleanOptional(value.file_library.s3.secret_key),
          }
        : undefined,
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
      task_timeout_secs: Number(payload.connection.task_timeout_secs),
      max_inflight: Number(payload.connection.max_inflight),
    },
    ocr_preset: (payload.ocr_preset ?? "").trim(),
    vlm: {
      openai_base_url: cleanOptional(vlm.openai_base_url),
      api_key: cleanOptional(vlm.api_key),
      vlm_pipeline_model: cleanOptional(vlm.vlm_pipeline_model),
      picture_description_model: cleanOptional(vlm.picture_description_model),
      picture_description_preset: cleanOptional(vlm.picture_description_preset),
      code_formula_model: cleanOptional(vlm.code_formula_model),
    },
  };
}

function cleanOptional(value: string | null | undefined) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
