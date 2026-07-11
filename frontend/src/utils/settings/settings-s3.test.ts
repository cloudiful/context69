import { describe, expect, it } from "vitest";

import { buildRuntimePayload, createRuntimeDraft } from "../settings";

describe("runtime S3 settings", () => {
  it("omits S3 when disabled", () => {
    const draft = createRuntimeDraft();
    expect(buildRuntimePayload(draft).file_library.s3).toBeUndefined();
  });

  it("normalizes enabled S3 settings without inventing a secret", () => {
    const draft = createRuntimeDraft();
    draft.file_library.s3_enabled = true;
    Object.assign(draft.file_library.s3, {
      endpoint: " https://s3.example.com ",
      region: " us-east-1 ",
      bucket: " context69 ",
      prefix: "/library/",
      path_style: true,
      access_key: " access ",
      secret_key: "",
    });

    expect(buildRuntimePayload(draft).file_library.s3).toEqual({
      endpoint: "https://s3.example.com",
      region: "us-east-1",
      bucket: "context69",
      prefix: "library",
      path_style: true,
      access_key: "access",
      secret_key: undefined,
    });
  });
});
