import type {
  DraftDoclingSettings,
  DraftRuntimeSettings,
  DraftSearchSettings,
} from "./settings-types";
import type {
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
  UpdateSearchSettingsRequest,
} from "../../services/api";
import { normalizeDoclingPayload, normalizeRuntimePayload } from "./settings-normalize";

export function buildRuntimePayload(
  draft: DraftRuntimeSettings,
): UpdateRuntimeSettingsRequest {
  return normalizeRuntimePayload(draft);
}

export function buildDoclingPayload(
  draft: DraftDoclingSettings,
): UpdateDoclingSettingsRequest {
  return normalizeDoclingPayload({
    connection: {
      base_url: draft.connection.base_url,
      timeout_secs: draft.connection.timeout_secs,
      poll_interval_secs: draft.connection.poll_interval_secs,
    },
    vlm: {
      openai_base_url: draft.vlm.openai_base_url,
      api_key: draft.vlm.api_key,
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
