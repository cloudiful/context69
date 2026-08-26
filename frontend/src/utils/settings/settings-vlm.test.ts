import { describe, expect, it } from "vitest";

import type { components } from "../../generated/openapi";
import type { DoclingSettingsResponse } from "../../services/api";
import {
  buildDoclingPayload,
  createDoclingDraft,
  doclingResponseToDraft,
  inferDoclingVlmMode,
  projectDoclingVlmForPayload,
} from "../../utils/settings";

type DoclingVlmSettingsResponse = components["schemas"]["DoclingVlmSettingsResponse"];

function makeDoclingResponse(
  vlmOverrides: Partial<DoclingVlmSettingsResponse> = {},
): DoclingSettingsResponse {
  return {
    configured: true,
    source: "database",
    connection: {
      base_url: "http://docling:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
    },
    vlm: {
      openai_base_url: null,
      has_api_key: false,
      vlm_pipeline_model: null,
      picture_description_model: null,
      code_formula_model: null,
      picture_description_preset: null,
      ...vlmOverrides,
    },
  };
}

describe("inferDoclingVlmMode", () => {
  it("treats a non-empty preset as preset mode even if a stored API key still exists", () => {
    expect(
      inferDoclingVlmMode({
        openai_base_url: null,
        has_api_key: true,
        vlm_pipeline_model: null,
        picture_description_model: null,
        code_formula_model: null,
        picture_description_preset: "granite_vision",
      }),
    ).toBe("preset");
  });

  it("treats whitespace-only preset as disabled and falls through to legacy detection", () => {
    expect(
      inferDoclingVlmMode({
        openai_base_url: null,
        has_api_key: false,
        vlm_pipeline_model: null,
        picture_description_model: null,
        code_formula_model: null,
        picture_description_preset: "   ",
      }),
    ).toBe("disabled");
  });

  it("treats a stored legacy bundle without preset as custom mode", () => {
    expect(
      inferDoclingVlmMode({
        openai_base_url: "https://openrouter.ai/api/v1",
        has_api_key: true,
        vlm_pipeline_model: "gemini",
        picture_description_model: "gpt-4o-mini",
        code_formula_model: "gpt-4o-mini",
        picture_description_preset: null,
      }),
    ).toBe("custom");
  });

  it("treats only-has-api-key as custom mode", () => {
    expect(
      inferDoclingVlmMode({
        openai_base_url: null,
        has_api_key: true,
        vlm_pipeline_model: null,
        picture_description_model: null,
        code_formula_model: null,
        picture_description_preset: null,
      }),
    ).toBe("custom");
  });

  it("treats a blank VLM response as disabled mode", () => {
    expect(
      inferDoclingVlmMode({
        openai_base_url: null,
        has_api_key: false,
        vlm_pipeline_model: null,
        picture_description_model: null,
        code_formula_model: null,
        picture_description_preset: null,
      }),
    ).toBe("disabled");
  });
});

describe("doclingResponseToDraft", () => {
  it("maps a preset-only response into preset mode", () => {
    const draft = doclingResponseToDraft(
      makeDoclingResponse({
        picture_description_preset: "smolvlm",
        has_api_key: false,
      }),
    );
    expect(draft.vlm_mode).toBe("preset");
    expect(draft.vlm.picture_description_preset).toBe("smolvlm");
  });

  it("maps a stored legacy bundle into custom mode", () => {
    const draft = doclingResponseToDraft(
      makeDoclingResponse({
        openai_base_url: "https://openrouter.ai/api/v1",
        has_api_key: true,
        vlm_pipeline_model: "gemini",
        picture_description_model: "gpt-4o-mini",
        code_formula_model: "gpt-4o-mini",
      }),
    );
    expect(draft.vlm_mode).toBe("custom");
    expect(draft.vlm.openai_base_url).toBe("https://openrouter.ai/api/v1");
    expect(draft.vlm.api_key).toBe("");
  });

  it("preserves preset value in the draft even when custom mode is selected", () => {
    const draft = doclingResponseToDraft(
      makeDoclingResponse({
        picture_description_preset: "smolvlm",
      }),
    );
    expect(draft.vlm_mode).toBe("preset");
    expect(draft.vlm.picture_description_preset).toBe("smolvlm");
  });
});

describe("projectDoclingVlmForPayload", () => {
  it("omits every VLM field in disabled mode", () => {
    const vlm = {
      openai_base_url: "https://api.example.com/v1",
      api_key: "sk-stored",
      vlm_pipeline_model: "model",
      picture_description_model: "pic",
      picture_description_preset: "preset",
      code_formula_model: "code",
    };
    expect(projectDoclingVlmForPayload("disabled", vlm)).toEqual({
      openai_base_url: undefined,
      api_key: undefined,
      vlm_pipeline_model: undefined,
      picture_description_model: undefined,
      picture_description_preset: undefined,
      code_formula_model: undefined,
    });
  });

  it("keeps only the preset in preset mode", () => {
    const vlm = {
      openai_base_url: "https://api.example.com/v1",
      api_key: "sk-stored",
      vlm_pipeline_model: "model",
      picture_description_model: "pic",
      picture_description_preset: "smolvlm",
      code_formula_model: "code",
    };
    expect(projectDoclingVlmForPayload("preset", vlm)).toEqual({
      openai_base_url: undefined,
      api_key: undefined,
      vlm_pipeline_model: undefined,
      picture_description_model: undefined,
      picture_description_preset: "smolvlm",
      code_formula_model: undefined,
    });
  });

  it("keeps the legacy bundle and drops the preset in custom mode", () => {
    const vlm = {
      openai_base_url: "https://api.example.com/v1",
      api_key: "sk-new",
      vlm_pipeline_model: "model",
      picture_description_model: "pic",
      picture_description_preset: "smolvlm",
      code_formula_model: "code",
    };
    expect(projectDoclingVlmForPayload("custom", vlm)).toEqual({
      openai_base_url: "https://api.example.com/v1",
      api_key: "sk-new",
      vlm_pipeline_model: "model",
      picture_description_model: "pic",
      picture_description_preset: undefined,
      code_formula_model: "code",
    });
  });
});

describe("buildDoclingPayload", () => {
  it("defaults to disabled VLM payload for a fresh draft", () => {
    const draft = createDoclingDraft();
    expect(draft.vlm_mode).toBe("disabled");
    const payload = buildDoclingPayload(draft);
    expect(payload.vlm).toEqual({
      openai_base_url: undefined,
      api_key: undefined,
      vlm_pipeline_model: undefined,
      picture_description_model: undefined,
      picture_description_preset: undefined,
      code_formula_model: undefined,
    });
  });

  it("projects preset mode even when the draft still holds stale legacy values", () => {
    const draft = createDoclingDraft();
    draft.vlm_mode = "preset";
    draft.vlm.picture_description_preset = "smolvlm";
    draft.vlm.openai_base_url = "https://api.example.com/v1";
    draft.vlm.api_key = "sk-stored";
    draft.vlm.vlm_pipeline_model = "model";
    draft.vlm.picture_description_model = "pic";
    draft.vlm.code_formula_model = "code";

    const payload = buildDoclingPayload(draft);
    const vlm = payload.vlm!;
    expect(vlm.picture_description_preset).toBe("smolvlm");
    expect(vlm.openai_base_url).toBeUndefined();
    expect(vlm.api_key).toBeUndefined();
    expect(vlm.vlm_pipeline_model).toBeUndefined();
    expect(vlm.picture_description_model).toBeUndefined();
    expect(vlm.code_formula_model).toBeUndefined();
  });

  it("drops preset in custom mode and keeps the legacy bundle", () => {
    const draft = createDoclingDraft();
    draft.vlm_mode = "custom";
    draft.vlm.openai_base_url = "https://api.example.com/v1";
    draft.vlm.api_key = "sk-new";
    draft.vlm.vlm_pipeline_model = "model";
    draft.vlm.picture_description_model = "pic";
    draft.vlm.code_formula_model = "code";
    draft.vlm.picture_description_preset = "should-be-cleared";

    const payload = buildDoclingPayload(draft);
    const vlm = payload.vlm!;
    expect(vlm.picture_description_preset).toBeUndefined();
    expect(vlm.openai_base_url).toBe("https://api.example.com/v1");
    expect(vlm.api_key).toBe("sk-new");
    expect(vlm.vlm_pipeline_model).toBe("model");
    expect(vlm.picture_description_model).toBe("pic");
    expect(vlm.code_formula_model).toBe("code");
  });
});
