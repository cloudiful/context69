import type {
  UpdateDoclingSettingsRequest,
  UpsertProviderAccountRequest,
} from "../../services/api";

export type DraftRuntimeSettings = {
  qdrant: {
    url: string;
    collection_name: string;
    recreate_on_dimension_mismatch: boolean;
  };
  embedding: {
    provider_account_key: string;
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
  };
};

export type DraftDoclingSettings = {
  connection: {
    base_url: string;
    timeout_secs: number;
    poll_interval_secs: number;
  };
  conversion: {
    pdf_backend: string;
    images_scale: number | null;
    image_export_mode: string;
  };
  ocr: {
    do_ocr: boolean;
    force_ocr: boolean;
    ocr_engine: string;
  };
  enrichment: {
    do_code_enrichment: boolean;
    do_formula_enrichment: boolean;
    do_picture_description: boolean;
  };
  vlm: {
    provider_account_key: string;
    vlm_pipeline_model: string;
    picture_description_model: string;
    code_formula_model: string;
  };
};

export type DraftDoclingSettingsEnvelope = {
  draft: DraftDoclingSettings;
  ocrLangText: string;
};

export type DoclingPayloadShape = Omit<UpdateDoclingSettingsRequest, "ocr"> & {
  ocr: UpdateDoclingSettingsRequest["ocr"] & {
    ocr_lang_text?: string;
  };
};

export type SettingsNavGroup = {
  key: string;
  label: string;
  items: Array<{ id: string; label: string }>;
};

export type DraftSearchSettings = {
  mode: "vector" | "hybrid";
  rerank_enabled: boolean;
  rerank_base_url: string;
  rerank_model: string;
  candidate_limit: number;
  timeout_secs: number;
};

export type ProviderAccountDraft = {
  account_key: string;
  provider_kind: string;
  display_name: string;
  base_url: string;
  api_key: string;
  clear_api_key: boolean;
  disabled: boolean;
};

export type ProviderAccountComparablePayload = UpsertProviderAccountRequest;
