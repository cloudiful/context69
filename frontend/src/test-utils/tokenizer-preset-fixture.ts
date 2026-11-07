import type { components } from "../generated/openapi";

/**
 * The one pinned tokenizer preset both test harnesses serve.
 *
 * It is a fixture of the generated descriptor type, so a contract change that
 * renames or drops a field fails type checking instead of silently drifting
 * away from the API.
 */
export const TOKENIZER_PRESET_FIXTURE: components["schemas"]["TokenizerPresetDescriptor"] = {
  id: "jina_v5_small_retrieval_r6856e76",
  label: "Jina embeddings v5 text small retrieval",
  sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
  size_bytes: 11422654,
  license: "cc-by-nc-4.0",
  license_url: "https://creativecommons.org/licenses/by-nc/4.0/",
  source_repo: "jinaai/jina-embeddings-v5-text-small-retrieval",
  source_url:
    "https://huggingface.co/jinaai/jina-embeddings-v5-text-small-retrieval/blob/6856e76bb72982e58de0620458a4e8b3614da340",
  revision: "6856e76bb72982e58de0620458a4e8b3614da340",
  ready: false,
};
