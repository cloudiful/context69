import type { components } from "../generated/openapi";

/**
 * The one pinned tokenizer preset both test harnesses serve.
 *
 * It is a fixture of the generated descriptor type, so a contract change that
 * renames or drops a field fails type checking instead of silently drifting
 * away from the API.
 *
 * It stays `ready: false` so the tests keep exercising the prepare flow; the
 * bundled image artifact is proven ready by the backend catalog fixture.
 */
export const TOKENIZER_PRESET_FIXTURE: components["schemas"]["TokenizerPresetDescriptor"] = {
  id: "embeddinggemma2_r_f9972ac",
  label: "EmbeddingGemma 2",
  sha256: "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
  size_bytes: 32170510,
  license: "apache-2.0",
  license_url: "https://www.apache.org/licenses/LICENSE-2.0",
  source_repo: "google/embeddinggemma-2",
  source_url:
    "https://huggingface.co/google/embeddinggemma-2/blob/f9972ac22e696a105c482202646979e33e4b5a1e",
  revision: "f9972ac22e696a105c482202646979e33e4b5a1e",
  ready: false,
};
