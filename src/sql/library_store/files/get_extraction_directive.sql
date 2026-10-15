SELECT extraction_template_key, extraction_parameters, extraction_required
FROM context69.library_files
WHERE id = $1
