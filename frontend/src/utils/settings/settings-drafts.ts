import type {
  DoclingSettingsResponse,
  RuntimeSettingsResponse,
  SearchSettingsResponse,
} from "../../services/api";
import type {
  DraftDoclingSettings,
  DraftRuntimeSettings,
  DraftSearchSettings,
} from "./settings-types";

export function createRuntimeDraft(): DraftRuntimeSettings {
  return {
    qdrant: {
      url: "",
      collection_name: "",
      recreate_on_dimension_mismatch: false,
    },
    embedding: {
      base_url: "",
      api_key: "",
      model: "",
      dimensions: 768,
      timeout_secs: 30,
    },
    scheduler: {
      interval_secs: 300,
      run_on_start: true,
      max_concurrency: 4,
      job_id: "context69-sync",
      valkey_url: "",
    },
    chunking: {
      max_chars: 1200,
      overlap_chars: 200,
    },
    file_library: {
      storage_root: "",
      max_upload_size_mb: 64,
      max_upload_request_size_mb: 128,
      ingest_concurrency: 2,
      pdf_pages_per_task: 5,
      s3_enabled: false,
      s3: {
        endpoint: "",
        region: "",
        bucket: "",
        prefix: "",
        path_style: false,
        access_key: "",
        secret_key: "",
      },
    },
  };
}

export function createDoclingDraft(): DraftDoclingSettings {
  return {
    connection: {
      base_url: "",
      timeout_secs: 120,
      poll_interval_secs: 2,
    },
    vlm: {
      openai_base_url: "",
      api_key: "",
      vlm_pipeline_model: "",
      picture_description_model: "",
      code_formula_model: "",
    },
  };
}

export function createSearchDraft(): DraftSearchSettings {
  return {
    mode: "hybrid",
    rerank_enabled: true,
    rerank_base_url: "https://openrouter.ai/api/v1",
    rerank_model: "cohere/rerank-4-fast",
    candidate_limit: 40,
    timeout_secs: 10,
  };
}

export function runtimeResponseToDraft(
  response: RuntimeSettingsResponse,
): DraftRuntimeSettings {
  return {
    qdrant: {
      url: response.qdrant.url,
      collection_name: response.qdrant.collection_name,
      recreate_on_dimension_mismatch: response.qdrant.recreate_on_dimension_mismatch,
    },
    embedding: {
      base_url: response.embedding.base_url,
      api_key: "",
      model: response.embedding.model,
      dimensions: response.embedding.dimensions,
      timeout_secs: response.embedding.timeout_secs,
    },
    scheduler: {
      interval_secs: response.scheduler.interval_secs,
      run_on_start: response.scheduler.run_on_start,
      max_concurrency: response.scheduler.max_concurrency,
      job_id: response.scheduler.job_id,
      valkey_url: response.scheduler.valkey_url ?? "",
    },
    chunking: {
      max_chars: response.chunking.max_chars,
      overlap_chars: response.chunking.overlap_chars,
    },
    file_library: {
      storage_root: response.file_library.storage_root,
      max_upload_size_mb: response.file_library.max_upload_size_mb,
      max_upload_request_size_mb: response.file_library.max_upload_request_size_mb,
      ingest_concurrency: response.file_library.ingest_concurrency,
      pdf_pages_per_task: response.file_library.pdf_pages_per_task,
      s3_enabled: !!response.file_library.s3,
      s3: {
        endpoint: response.file_library.s3?.endpoint ?? "",
        region: response.file_library.s3?.region ?? "",
        bucket: response.file_library.s3?.bucket ?? "",
        prefix: response.file_library.s3?.prefix ?? "",
        path_style: response.file_library.s3?.path_style ?? false,
        access_key: response.file_library.s3?.access_key ?? "",
        secret_key: "",
      },
    },
  };
}

export function doclingResponseToDraft(
  response: DoclingSettingsResponse,
): DraftDoclingSettings {
  return {
    connection: {
      base_url: response.connection.base_url ?? "",
      timeout_secs: response.connection.timeout_secs,
      poll_interval_secs: response.connection.poll_interval_secs,
    },
    vlm: {
      openai_base_url: response.vlm.openai_base_url ?? "",
      api_key: "",
      vlm_pipeline_model: response.vlm.vlm_pipeline_model ?? "",
      picture_description_model: response.vlm.picture_description_model ?? "",
      code_formula_model: response.vlm.code_formula_model ?? "",
    },
  };
}

export function searchResponseToPayload(response: SearchSettingsResponse): DraftSearchSettings {
  return {
    mode: response.mode,
    rerank_enabled: response.rerank_enabled,
    rerank_base_url: response.rerank_base_url,
    rerank_model: response.rerank_model,
    candidate_limit: response.candidate_limit,
    timeout_secs: response.timeout_secs,
  };
}
