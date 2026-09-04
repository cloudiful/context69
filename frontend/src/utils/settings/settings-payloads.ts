import type {
  DraftDoclingSettings,
  DraftDoclingVlmMode,
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
      task_timeout_secs: draft.connection.task_timeout_secs,
      max_inflight: draft.connection.max_inflight,
    },
    vlm: projectDoclingVlmForPayload(draft.vlm_mode, draft.vlm),
  });
}

/**
 * Project the draft VLM fields onto only the values the backend should see
 * for the active mode. The draft keeps every field the user has touched so
 * switching modes is non-destructive, but a save in `disabled` or `preset`
 * mode must drop the legacy bundle (and clear the stored API key on the
 * backend by omission), and a save in `preset` mode must clear the legacy
 * fields as well.
 */
export function projectDoclingVlmForPayload(
  mode: DraftDoclingVlmMode,
  vlm: DraftDoclingSettings["vlm"],
): UpdateDoclingSettingsRequest["vlm"] {
  switch (mode) {
    case "disabled":
      return {
        openai_base_url: undefined,
        api_key: undefined,
        vlm_pipeline_model: undefined,
        picture_description_model: undefined,
        picture_description_preset: undefined,
        code_formula_model: undefined,
      };
    case "preset":
      return {
        openai_base_url: undefined,
        api_key: undefined,
        vlm_pipeline_model: undefined,
        picture_description_model: undefined,
        picture_description_preset: vlm.picture_description_preset,
        code_formula_model: undefined,
      };
    case "custom":
      return {
        openai_base_url: vlm.openai_base_url,
        api_key: vlm.api_key,
        vlm_pipeline_model: vlm.vlm_pipeline_model,
        picture_description_model: vlm.picture_description_model,
        picture_description_preset: undefined,
        code_formula_model: vlm.code_formula_model,
      };
  }
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
