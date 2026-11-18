import type { components } from "../../generated/openapi";
import type {
  SearchMode,
  UpdateDoclingSettingsRequest,
} from "../../services/api";

type RuntimeChunkingSettings = components["schemas"]["RuntimeChunkingSettings"];

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
  };
  /**
   * The optional Git code embedding draft.
   *
   * Every provider field is an override: a blank one inherits the document
   * configuration field by field. `touched` is frontend-only: an ordinary
   * document save omits the block so it can never disable code embedding,
   * while any code edit carries it.
   */
  code_embedding: {
    enabled: boolean;
    base_url: string;
    api_key: string;
    model: string;
    /** The model code queries are embedded with; blank inherits the code model. */
    query_model: string;
    /** The dimension override; null inherits the document dimensions. */
    dimensions: number | null;
    /** The timeout override; null inherits the document timeout. */
    timeout_secs: number | null;
    /** Whether the response reported a stored code credential. */
    has_api_key: boolean;
    /** Whether the credential in effect is the inherited document credential. */
    inherits_document_api_key: boolean;
    /** An explicit request to remove the stored code credential. */
    clear_api_key: boolean;
    touched: boolean;
  };
  /**
   * The optional image embedding draft.
   *
   * Every provider field is an override: a blank one inherits the document
   * configuration field by field, while the collection name falls back to the
   * default image namespace. `touched` is frontend-only: an ordinary document
   * or code save omits the block so it can never disable image embedding,
   * while any image edit carries it. The collection name is a namespace, not
   * a physical collection: the backend derives the physical collection from
   * the runtime fingerprint.
   */
  image_embedding: {
    enabled: boolean;
    base_url: string;
    api_key: string;
    /** The model stored image bytes are embedded with. */
    model: string;
    /** The model text queries are embedded with; blank inherits the image model. */
    query_model: string;
    /** The dimension override; null inherits the document dimensions. */
    dimensions: number | null;
    /** The timeout override; null inherits the document timeout. */
    timeout_secs: number | null;
    /** The namespace image vectors live under. */
    collection_name: string;
    /** Whether the response reported a stored image credential. */
    has_api_key: boolean;
    /** Whether the credential in effect is the inherited document credential. */
    inherits_document_api_key: boolean;
    /** An explicit request to remove the stored image credential. */
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
  /** Whether PDF and DOCX conversions request OCR. */
  do_ocr: boolean;
  /** Whether PDF and DOCX conversions request picture description. */
  do_picture_description: boolean;
  /** Whether PDF and DOCX conversions request code enrichment. */
  do_code_enrichment: boolean;
  /** Whether PDF and DOCX conversions request formula enrichment. */
  do_formula_enrichment: boolean;
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
