import { describe, expect, it } from "vitest";

import type { DoclingSettingsResponse } from "../../services/api";
import {
  DEFAULT_DOCLING_OCR_PRESET,
  buildDoclingPayload,
  createDoclingDraft,
  doclingResponseToDraft,
  doclingResponseToPayload,
} from "../../utils/settings";

function makeDoclingResponse(ocrPreset: string): DoclingSettingsResponse {
  return {
    configured: true,
    source: "database",
    connection: {
      base_url: "http://docling:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      max_inflight: 2,
    },
    ocr_preset: ocrPreset,
    vlm: {
      has_api_key: false,
      openai_base_url: null,
      vlm_pipeline_model: null,
      picture_description_model: null,
      picture_description_preset: null,
      code_formula_model: null,
    },
  };
}

describe("the docling OCR preset", () => {
  it("defaults a fresh draft to the default preset", () => {
    expect(DEFAULT_DOCLING_OCR_PRESET).toBe("rapidocr");
    expect(createDoclingDraft().ocr_preset).toBe("rapidocr");
  });

  it("shows the stored preset in the draft", () => {
    expect(doclingResponseToDraft(makeDoclingResponse("rapidocr")).ocr_preset).toBe("rapidocr");
    expect(doclingResponseToDraft(makeDoclingResponse("auto")).ocr_preset).toBe("auto");
  });

  it("sends the selected preset with the settings payload", () => {
    const draft = doclingResponseToDraft(makeDoclingResponse("rapidocr"));
    expect(buildDoclingPayload(draft).ocr_preset).toBe("rapidocr");
  });

  it("round-trips a custom server preset", () => {
    const draft = doclingResponseToDraft(makeDoclingResponse("custom.ocr-preset"));
    draft.ocr_preset = "custom.ocr-preset";
    expect(buildDoclingPayload(draft).ocr_preset).toBe("custom.ocr-preset");
  });

  it("trims the selected preset", () => {
    const draft = createDoclingDraft();
    draft.ocr_preset = "  tesseract  ";
    expect(buildDoclingPayload(draft).ocr_preset).toBe("tesseract");
  });

  it("keeps an emptied preset visible so the backend can reject it", () => {
    const draft = createDoclingDraft();
    draft.ocr_preset = "";
    expect(buildDoclingPayload(draft).ocr_preset).toBe("");
  });

  it("compares an unchanged preset as no change", () => {
    const response = makeDoclingResponse("rapidocr");
    expect(doclingResponseToPayload(response).ocr_preset).toBe("rapidocr");
  });
});
