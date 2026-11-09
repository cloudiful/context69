import type { components } from "../../generated/openapi";
import type {
  EmbeddingBatchingSettings,
  SearchMode,
  UpdateDoclingSettingsRequest,
} from "../../services/api";

type RuntimeChunkingSettings = components["schemas"]["RuntimeChunkingSettings"];

/**
 * The manual limits of the embedding request planner.
 *
 * Every generated field is present, so a save always carries the limits the
 * operator sees instead of dropping them.
 */
export type DraftEmbeddingBatching = Required<EmbeddingBatchingSettings>;

/** The explicit embedding input envelope of the runtime draft. */
export type DraftEmbeddingInputPolicy = {
  /** A null cap disables the guard. */
  max_input_tokens: number | null;
  document_server_prefix: string;
  query_server_prefix: string;
  /**
   * Whether the counted input includes the tokenizer's special tokens.
   *
   * The tokenizer itself is not part of this draft: the lower chunking
   * selector owns it, and the request derives it from that selection.
   */
  add_special_tokens: boolean;
};

/**
 * Chunking budget of the runtime draft.
 *
 * Every generated field is present, so switching the unit never drops the
 * inactive budget instead of reinterpreting it.
 */
export type DraftChunkingSettings = Required<RuntimeChunkingSettings>;

/** Unit the chunking budget is measured in. */
export type DraftChunkingUnit = NonNullable<RuntimeChunkingSettings["unit"]>;

/** Tokenizer that measures the token budget. */
export type DraftChunkingTokenizer = NonNullable<RuntimeChunkingSettings["tokenizer"]>;

export type DraftRuntimeSettings = {
  qdrant: {
    url: string;
    collection_name: string;
    recreate_on_dimension_mismatch: boolean;
  };
  embedding: {
    base_url: string;
    api_key: string;
    /** The model documents are embedded with. */
    model: string;
    /** The model queries are embedded with; blank inherits the document model. */
    query_model: string;
    dimensions: number;
    timeout_secs: number;
    batching: DraftEmbeddingBatching;
    input_policy: DraftEmbeddingInputPolicy;
  };
  /**
   * The independent optional Git code embedding draft.
   *
   * `touched` is frontend-only: an ordinary document save omits the block so it
   * can never disable code embedding, while any code edit carries it.
   */
  code_embedding: {
    enabled: boolean;
    base_url: string;
    api_key: string;
    model: string;
    /** The model code queries are embedded with; blank inherits the code model. */
    query_model: string;
    dimensions: number;
    timeout_secs: number;
    batching: DraftEmbeddingBatching;
    /** Whether the response reported a stored code credential. */
    has_api_key: boolean;
    /** An explicit request to remove the stored code credential. */
    clear_api_key: boolean;
    touched: boolean;
  };
  scheduler: {
    interval_secs: number;
    run_on_start: boolean;
    max_concurrency: number;
    job_id: string;
    valkey_url: string;
  };
  chunking: DraftChunkingSettings;
  file_library: {
    storage_root: string;
    max_upload_size_mb: number;
    max_upload_request_size_mb: number;
    ingest_concurrency: number;
    url_import_concurrency: number;
    url_import_min_interval_ms: number;
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

export type DraftDoclingVlmMode = "disabled" | "preset" | "custom";

export type DraftDoclingSettings = {
  connection: {
    base_url: string;
    timeout_secs: number;
    poll_interval_secs: number;
    task_timeout_secs: number;
    max_inflight: number;
  };
  /** The Docling OCR preset every PDF and DOCX conversion requests. */
  ocr_preset: string;
  /** Frontend-only draft mode selector; never persisted to the backend. */
  vlm_mode: DraftDoclingVlmMode;
  vlm: {
    openai_base_url: string;
    api_key: string;
    vlm_pipeline_model: string;
    picture_description_model: string;
    picture_description_preset: string;
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
  /** Hybrid fusion weight for the semantic/vector channel (0..1). */
  vector_weight: number;
  /** Hybrid fusion weight for the keyword channel (0..1). */
  keyword_weight: number;
};

export type DoclingPayloadShape = UpdateDoclingSettingsRequest;
