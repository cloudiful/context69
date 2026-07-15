SELECT
    base_url,
    timeout_secs,
    poll_interval_secs,
    task_timeout_secs,
    pdf_backend,
    images_scale,
    image_export_mode,
    do_ocr,
    force_ocr,
    ocr_engine,
    ocr_lang,
    do_code_enrichment,
    do_formula_enrichment,
    do_picture_description,
    openai_base_url,
    api_key,
    vlm_pipeline_model,
    picture_description_model,
    code_formula_model
FROM context69.docling_settings
WHERE singleton = TRUE
