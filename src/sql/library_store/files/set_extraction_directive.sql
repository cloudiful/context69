UPDATE context69.library_files
SET extraction_template_key = $2,
    extraction_parameters = $3,
    updated_at = now()
WHERE id = $1

