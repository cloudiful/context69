# Extraction decision templates — contract migration guide (issue #89 P4)

Public template and result contracts are now tagged unions. This is an
accepted breaking change to request/response JSON; stored data and the
generative execution path are preserved.

## Template input (PUT extraction-templates)

Old flat shape is rejected; send one tagged definition:

```json
{
  "template_key": "t-1",
  "description": null,
  "enabled": true,
  "definition": {
    "kind": "extraction",
    "system_prompt": "p",
    "output_schema": {"type": "object"},
    "max_output_tokens": null
  }
}
```

Decision templates use the other branch:

```json
{
  "template_key": "d-1",
  "definition": {
    "kind": "decision",
    "questions": [
      {"type": "predicate", "name": "relevant", "instructions": "Is it relevant?"},
      {"type": "choice", "name": "direction", "instructions": "Pick one.",
       "choices": [
         {"value": "up", "description": "Upward."},
         {"value": "down", "description": "Downward."}
       ]},
      {"type": "score", "name": "quality", "instructions": "Rate it.",
       "levels": [
         {"label": "bad", "description": "Bad."},
         {"label": "good", "description": "Good."}
       ]}
  }
}
```

Question kinds: `predicate{name, instructions}`, `choice{name, instructions,
choices[{value, description}]}`, `score{name, instructions, levels[{label,
description}]}`. Application limits (Context69 limits, not vendor claims):
1–32 questions per template, 2–64 options per choice, 2–32 levels per
score; names/values/labels unique and nonempty; instructions and option
descriptions nonempty. Unknown `kind`/`type` values and unknown fields are
rejected; there is no silent flat/nested dual parsing.

Until the decision adapter phase lands, registering a decision template
succeeds but any job on it finishes `failed` (permanent,
`decision provider is not configured`) before any provider call. It never
falls back to generative chat and never succeeds with a stub.

## Result payload (list/rebuild responses)

`result_json` is replaced by the tagged `result`:

```json
{"kind": "extraction", "value": {"direction": "positive"}}
{"kind": "decision", "answers": [
  {"type": "predicate", "name": "relevant", "probability": 0.7}
]}
```

Answer kinds: `predicate{name, probability}`,
`choice{name, choice, probabilities[{value, probability}], confidence}`,
`score{name, score, probabilities[{value, label, probability}], confidence}`
(`value` is the zero-based level index), and `refusal{name}`, which decodes
as failure evidence and never counts as a successful persisted decision.
Identity/version/hash/model/time fields are unchanged.

## Stored data compatibility

- Existing template rows default to `extraction` and keep their exact
  prompt/schema/token cap; they list and execute generatively.
- Each result row snapshots its kind at publish time. Changing a template's
  kind (which bumps its version) never reinterprets already-persisted
  results: history decodes from the row snapshot, not the live template.
- Internal `result_json` storage and the `metadata[extraction][key]`
  namespace are unchanged.

## SDK upgrade

`ExtractionTemplateInput`/`ExtractionTemplateResponse`/
`ExtractionResultResponse` keep their names but carry the new shapes; the
new definition/question/answer/payload types are re-exported from the SDK
facade. No endpoint, method, or operation changed (137 operations).
Regenerate or update any handwritten copies of these three schemas; the
generated frontend pair (`frontend/openapi/context69.openapi.json`,
`frontend/src/generated/openapi.ts`) is the authoritative reference.
