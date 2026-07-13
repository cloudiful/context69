import type {
  SearchMode,
  UpdateDoclingSettingsRequest,
} from "../../services/api";

export type DraftRuntimeSettings = {
  qdrant: {
    url: string;
    collection_name: string;
    recreate_on_dimension_mismatch: boolean;
  };
  embedding: {
    base_url: string;
    api_key: string;
    model: string;
    dimensions: number;
    timeout_secs: number;
  };
  scheduler: {
    interval_secs: number;
    run_on_start: boolean;
    max_concurrency: number;
    job_id: string;
    valkey_url: string;
  };
  chunking: {
    max_chars: number;
    overlap_chars: number;
  };
  file_library: {
    storage_root: string;
    max_upload_size_mb: number;
    max_upload_request_size_mb: number;
    ingest_concurrency: number;
    pdf_pages_per_task: number;
    trusted_proxy_enabled: boolean;
    s3_enabled: boolean;
    s3: {
      endpoint: string;
      region: string;
      bucket: string;
      prefix: string;
      path_style: boolean;
      access_key: string;
      secret_key: string;
    };
  };
};

export type DraftDoclingSettings = {
  connection: {
    base_url: string;
    timeout_secs: number;
    poll_interval_secs: number;
  };
  vlm: {
    openai_base_url: string;
    api_key: string;
    vlm_pipeline_model: string;
    picture_description_model: string;
    code_formula_model: string;
  };
};

export type DraftSearchSettings = {
  mode: SearchMode;
  rerank_enabled: boolean;
  rerank_base_url: string;
  rerank_model: string;
  candidate_limit: number;
  timeout_secs: number;
};

export type DoclingPayloadShape = UpdateDoclingSettingsRequest;
