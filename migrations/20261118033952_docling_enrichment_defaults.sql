-- Truthful Docling enrichment defaults for existing settings.
--
-- The previous runtime forced `do_code_enrichment = true` and
-- `do_formula_enrichment = true` into the stored row. Neither value was an
-- operator control: the client only forwarded them with a complete legacy VLM
-- bundle while picture description was enabled. This migration makes the stored
-- flags describe what the deployment actually did.
--
-- Code enrichment was never independently selectable, and the user explicitly
-- requested it off, so every existing row becomes `false`.
--
-- Formula enrichment preserves only a configuration that the previous client
-- could actually have applied. That requires picture description to be enabled
-- and the complete legacy bundle `resolve_vlm_runtime_config` reads: a
-- non-blank `openai_base_url`, `vlm_pipeline_model`, `picture_description_model`,
-- and `code_formula_model`, together with a readable provider credential. Each
-- field is checked with the same Unicode White_Space set Rust `str::trim`
-- removes, so a field holding only a tab or a no-break space is not configured.
--
-- The credential lives only in the encrypted secret store now, so its stored
-- readability is checked without decrypting or printing anything. A legacy
-- plaintext row (`ciphertext_version = 0`, purpose NULL) stays readable when
-- its bytes decode lossily to a non-blank value. The check inspects the hex
-- representation instead of decoding it, so bytes that are not valid UTF-8
-- cannot abort the migration: they decode to replacement characters in the
-- store and therefore count as non-blank here too. The hex alternatives below
-- are the UTF-8 encodings of exactly the White_Space set used above. A sealed
-- row is readable when it carries the Docling purpose and a nonempty ciphertext.
-- Every other row becomes `false`.
UPDATE context69.docling_settings
SET do_code_enrichment = FALSE,
    do_formula_enrichment = CASE
        WHEN do_formula_enrichment
            AND do_picture_description
            AND btrim(COALESCE(openai_base_url, ''), U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> ''
            AND btrim(COALESCE(vlm_pipeline_model, ''), U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> ''
            AND btrim(COALESCE(picture_description_model, ''), U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> ''
            AND btrim(COALESCE(code_formula_model, ''), U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> ''
            AND EXISTS (
                SELECT 1
                FROM context69.internal_secrets AS secret
                WHERE secret.key = 'docling_vlm_api_key'
                    AND (
                        (
                            secret.ciphertext_version = 0
                            AND encode(secret.value, 'hex') !~ '^(09|0a|0b|0c|0d|20|c285|c2a0|e19a80|e2808[0-9a]|e280a8|e280a9|e280af|e2819f|e38080)*$'
                        )
                        OR (
                            secret.purpose = 'docling.vlm_api_key'
                            AND octet_length(secret.value) > 0
                        )
                    )
            )
        THEN TRUE
        ELSE FALSE
    END
WHERE singleton;
