pub mod docling_settings;
pub mod document_chunks;
pub mod documents;
pub mod git_provider_connections;
pub mod git_repository_sources;
pub mod git_webhook_registrations;
pub mod group_memberships;
pub mod groups;
pub mod library_dependency_gates;
pub mod library_file_documents;
pub mod library_files;
pub mod library_folders;
pub mod library_legacy_object_cleanup;
pub mod library_storage_objects;
pub mod personal_access_tokens;
pub mod rerank_item_scores;
pub mod runtime_chunking_settings;
pub mod runtime_code_embedding_settings;
pub mod runtime_embedding_settings;
pub mod runtime_file_library_settings;
pub mod runtime_qdrant_settings;
pub mod runtime_scheduler_settings;
pub mod runtime_source_connections;
pub mod search_generations;
pub mod search_settings;
pub mod source_checkpoints;
pub mod source_configs;
pub mod sync_runs;
pub mod users;

#[cfg(test)]
mod tests {
    use super::{
        docling_settings, document_chunks, documents, git_repository_sources,
        git_webhook_registrations, group_memberships, groups, library_dependency_gates,
        library_file_documents, library_files, library_folders, library_legacy_object_cleanup,
        library_storage_objects, personal_access_tokens, rerank_item_scores,
        runtime_chunking_settings, runtime_code_embedding_settings, runtime_embedding_settings,
        runtime_file_library_settings, runtime_qdrant_settings, runtime_scheduler_settings,
        runtime_source_connections, search_generations, search_settings, source_checkpoints,
        source_configs, sync_runs, users,
    };
    use sea_orm::sea_query::PostgresQueryBuilder;

    fn rendered(schema: sea_orm::sea_query::TableCreateStatement) -> String {
        schema.to_string(PostgresQueryBuilder)
    }

    #[test]
    fn declarations_cover_groups_sources_and_sync() {
        let groups = rendered(groups::schema());
        assert!(
            groups.contains("\"id\" bigserial NOT NULL PRIMARY KEY"),
            "{groups}"
        );
        assert!(
            groups.contains("\"default_source_policy\" text NOT NULL DEFAULT 'retain'"),
            "{groups}"
        );
        let group_unique = groups::parent_group_key_unique_index().to_string(PostgresQueryBuilder);
        assert!(
            group_unique.contains("COALESCE(parent_group_id, 0)"),
            "{group_unique}"
        );
        assert!(group_unique.contains("UNIQUE"), "{group_unique}");
        assert!(
            groups.contains("CONSTRAINT \"uq_groups_full_path\" UNIQUE (\"full_path\")"),
            "{groups}"
        );
        let memberships = rendered(group_memberships::schema());
        assert!(
            memberships.contains("PRIMARY KEY (\"group_id\", \"user_id\")"),
            "{memberships}"
        );
        let sources = rendered(source_configs::schema());
        assert!(
            sources.contains("\"example_queries\" jsonb NOT NULL DEFAULT ('[]'::jsonb)"),
            "{sources}"
        );
        let checkpoints = rendered(source_checkpoints::schema());
        assert!(
            checkpoints.contains("\"source_key\" text NOT NULL PRIMARY KEY"),
            "{checkpoints}"
        );
        let runs = rendered(sync_runs::schema());
        assert!(
            runs.contains("\"id\" bigserial NOT NULL PRIMARY KEY"),
            "{runs}"
        );
        assert!(
            runs.contains("\"records_seen\" integer NOT NULL DEFAULT 0"),
            "{runs}"
        );
    }

    #[test]
    fn declarations_cover_users_and_source_connections() {
        let users = rendered(users::schema());
        assert!(users.contains("\"context69\".\"users\""), "{users}");
        assert!(users.contains("bigserial"), "{users}");
        assert!(
            users.contains("\"disabled_at\" timestamp with time zone"),
            "{users}"
        );
        assert!(
            users.contains("\"login_name\" text NOT NULL UNIQUE"),
            "{users}"
        );
        assert!(!users.contains("CREATE UNIQUE INDEX"), "{users}");
        let connections = rendered(runtime_source_connections::schema());
        assert!(
            connections.contains("\"connection_key\" uuid NOT NULL DEFAULT (gen_random_uuid())"),
            "{connections}"
        );
        assert!(
            connections.contains("CHECK (btrim(name) <> '')"),
            "{connections}"
        );
        assert!(
            connections.contains(
                "CONSTRAINT \"uq_runtime_source_connections_connection_key\" UNIQUE (\"connection_key\")"
            ),
            "{connections}"
        );
        assert!(
            !connections.contains("CREATE UNIQUE INDEX"),
            "{connections}"
        );
    }

    #[test]
    fn declarations_cover_search_cache_ddl() {
        let generations = rendered(search_generations::schema());
        let scores = rendered(rerank_item_scores::schema());
        let index = rerank_item_scores::last_used_index().to_string(PostgresQueryBuilder);
        assert!(generations.contains("\"context69\".\"search_generations\""));
        assert!(
            generations
                .to_lowercase()
                .contains("timestamp with time zone not null default current_timestamp"),
            "{generations}"
        );
        assert!(scores.contains("\"context69\".\"rerank_item_scores\""));
        assert!(scores.contains("PRIMARY KEY (\"rerank_model\", \"query_hash\", \"chunk_id\")"));
        assert!(
            scores.contains("\"score\" real NOT NULL")
                || scores.contains("\"score\" REAL NOT NULL")
        );
        assert!(index.contains("rerank_item_scores_last_used_at_idx"));
    }

    #[test]
    fn declarations_cover_settings_ddl() {
        let search = rendered(search_settings::schema());
        assert!(search.contains("\"context69\".\"search_settings\""));
        assert!(search.contains("CHECK (singleton)"), "{search}");
        assert!(
            search.contains("CHECK (mode IN ('vector', 'hybrid'))"),
            "{search}"
        );
        assert!(
            search.contains("\"vector_weight\" real NOT NULL DEFAULT 0.55"),
            "{search}"
        );
        assert!(
            search.contains("\"keyword_weight\" real NOT NULL DEFAULT 0.35"),
            "{search}"
        );

        let docling = rendered(docling_settings::schema());
        assert!(docling.contains("\"context69\".\"docling_settings\""));
        assert!(
            docling.contains("\"ocr_lang\" text[] NOT NULL"),
            "{docling}"
        );
        assert!(
            docling.contains("CHECK (max_inflight >= 1 AND max_inflight <= 32)"),
            "{docling}"
        );
        assert!(
            docling.contains("\"task_timeout_secs\" bigint NOT NULL DEFAULT 600"),
            "{docling}"
        );

        let qdrant = rendered(runtime_qdrant_settings::schema());
        assert!(qdrant.contains("\"context69\".\"runtime_qdrant_settings\""));
        assert!(
            qdrant.contains("CHECK (btrim(collection_name) <> '')"),
            "{qdrant}"
        );

        let embedding = rendered(runtime_embedding_settings::schema());
        assert!(embedding.contains("\"context69\".\"runtime_embedding_settings\""));
        assert!(embedding.contains("CHECK (dimensions > 0)"), "{embedding}");

        let scheduler = rendered(runtime_scheduler_settings::schema());
        assert!(scheduler.contains("\"context69\".\"runtime_scheduler_settings\""));
        assert!(scheduler.contains("\"valkey_url\" text"), "{scheduler}");

        let chunking = rendered(runtime_chunking_settings::schema());
        assert!(chunking.contains("\"context69\".\"runtime_chunking_settings\""));
        assert!(
            chunking.contains("CHECK (unit IN ('chars', 'tokens'))"),
            "{chunking}"
        );
        assert!(
            chunking.contains(
                "CHECK (overlap_tokens >= 0 AND (unit <> 'tokens' OR overlap_tokens < max_tokens))"
            ),
            "{chunking}"
        );

        let file_library = rendered(runtime_file_library_settings::schema());
        assert!(file_library.contains("\"context69\".\"runtime_file_library_settings\""));
        assert!(
            file_library.contains("CHECK (registry_max_ids_per_run <= registry_max_ids_per_file)"),
            "{file_library}"
        );

        let code = rendered(runtime_code_embedding_settings::schema());
        assert!(code.contains("\"context69\".\"runtime_code_embedding_settings\""));
        assert!(code.contains("NOT enabled OR ("), "{code}");
        assert!(code.contains("CHECK (singleton)"), "{code}");

        for settings in [
            rendered(search_settings::schema()),
            rendered(docling_settings::schema()),
            rendered(runtime_qdrant_settings::schema()),
            rendered(runtime_embedding_settings::schema()),
            rendered(runtime_scheduler_settings::schema()),
            rendered(runtime_chunking_settings::schema()),
            rendered(runtime_file_library_settings::schema()),
            rendered(runtime_code_embedding_settings::schema()),
        ] {
            assert!(
                settings.contains("\"singleton\" bool NOT NULL DEFAULT TRUE PRIMARY KEY"),
                "{settings}"
            );
        }
    }

    #[test]
    fn standalone_unique_indexes_match_the_applied_unique_indexes() {
        let documents = documents::group_source_external_index().to_string(PostgresQueryBuilder);
        assert!(documents.starts_with("CREATE UNIQUE INDEX"), "{documents}");
        assert!(
            documents.contains("uq_documents_group_source_external_id"),
            "{documents}"
        );
        assert!(
            documents.contains("\"group_id\", \"source_key\", \"external_id\""),
            "{documents}"
        );

        let folders =
            library_folders::group_parent_name_unique_index().to_string(PostgresQueryBuilder);
        assert!(folders.starts_with("CREATE UNIQUE INDEX"), "{folders}");
        assert!(
            folders.contains("uq_library_folders_group_parent_name"),
            "{folders}"
        );

        let files =
            library_files::group_folder_filename_unique_index().to_string(PostgresQueryBuilder);
        assert!(files.starts_with("CREATE UNIQUE INDEX"), "{files}");
        assert!(
            files.contains("uq_library_files_group_folder_filename"),
            "{files}"
        );
        assert!(
            files.contains("COALESCE(folder_id::text, '__root__')"),
            "{files}"
        );

        let external =
            library_files::group_external_id_unique_index().to_string(PostgresQueryBuilder);
        assert!(external.starts_with("CREATE UNIQUE INDEX"), "{external}");
        assert!(
            external.contains("uq_library_files_group_external_id"),
            "{external}"
        );
        assert!(
            external.contains("WHERE external_id IS NOT NULL"),
            "{external}"
        );

        let webhooks =
            git_webhook_registrations::hook_unique_index().to_string(PostgresQueryBuilder);
        assert!(webhooks.starts_with("CREATE UNIQUE INDEX"), "{webhooks}");
        assert!(
            webhooks.contains("uq_git_webhook_registrations_hook"),
            "{webhooks}"
        );
        assert!(
            webhooks.contains("\"provider_kind\", \"external_hook_id\""),
            "{webhooks}"
        );
    }

    #[test]
    fn declarations_cover_document_chunk_ddl() {
        let chunks = rendered(document_chunks::schema());
        assert!(
            chunks.contains("\"context69\".\"document_chunks\""),
            "{chunks}"
        );
        assert!(
            chunks.contains("\"id\" uuid NOT NULL PRIMARY KEY"),
            "{chunks}"
        );
        assert!(
            chunks.contains(
                "CONSTRAINT \"document_chunks_document_id_chunk_index_key\" UNIQUE (\"document_id\", \"chunk_index\")"
            ),
            "{chunks}"
        );
        assert!(
            !chunks.contains("idx_document_chunks_document_id"),
            "{chunks}"
        );

        let record_hash = document_chunks::record_hash_index().to_string(PostgresQueryBuilder);
        assert!(
            record_hash.contains("idx_document_chunks_record_hash"),
            "{record_hash}"
        );
    }

    #[test]
    fn declarations_cover_git_repository_source_ddl() {
        let sources = rendered(git_repository_sources::schema());
        assert!(
            sources.contains("\"context69\".\"git_repository_sources\""),
            "{sources}"
        );
        assert!(
            sources.contains("\"cleanup_deferred_until\" timestamp with time zone"),
            "{sources}"
        );
        assert!(
            sources.contains(
                "CONSTRAINT \"uq_git_repository_sources_group_ref\" UNIQUE (\"group_id\", \"canonical_url\", \"target_ref\")"
            ),
            "{sources}"
        );
    }

    #[test]
    fn declarations_cover_library_folder_ddl() {
        let folders = rendered(library_folders::schema());
        assert!(
            folders.contains("\"context69\".\"library_folders\""),
            "{folders}"
        );
        assert!(
            folders.contains("\"id\" uuid NOT NULL PRIMARY KEY"),
            "{folders}"
        );
        assert!(
            folders.contains("CHECK (visibility IN ('public', 'private'))"),
            "{folders}"
        );
        assert!(folders.contains("CHECK (btrim(name) <> '')"), "{folders}");
        assert!(
            folders.contains("\"group_id\" bigint NOT NULL"),
            "{folders}"
        );

        let unique =
            library_folders::group_parent_name_unique_index().to_string(PostgresQueryBuilder);
        assert!(
            unique.contains("uq_library_folders_group_parent_name"),
            "{unique}"
        );
        assert!(
            unique.contains("COALESCE(parent_id::text, '__root__')"),
            "{unique}"
        );
        assert!(unique.contains("UNIQUE"), "{unique}");

        let pagination =
            library_folders::group_parent_updated_at_index().to_string(PostgresQueryBuilder);
        assert!(
            pagination.contains("idx_library_folders_group_parent_updated_at"),
            "{pagination}"
        );
        assert!(pagination.contains("\"updated_at\" DESC"), "{pagination}");
    }

    #[test]
    fn declarations_cover_library_file_ddl() {
        let files = rendered(library_files::schema());
        assert!(files.contains("\"context69\".\"library_files\""), "{files}");
        assert!(
            files.contains("\"id\" uuid NOT NULL PRIMARY KEY"),
            "{files}"
        );
        assert!(
            files.contains("\"metadata_json\" jsonb NOT NULL DEFAULT ('{}'::jsonb)"),
            "{files}"
        );
        assert!(
            files.contains(
                "\"translation_target_locales\" text[] NOT NULL DEFAULT (ARRAY[]::TEXT[])"
            ),
            "{files}"
        );
        assert!(
            files.contains("\"delete_source_after_processing\" bool NOT NULL DEFAULT FALSE"),
            "{files}"
        );
        assert!(files.contains("CHECK (size_bytes >= 0)"), "{files}");

        let unique =
            library_files::group_external_id_unique_index().to_string(PostgresQueryBuilder);
        assert!(
            unique.contains("uq_library_files_group_external_id"),
            "{unique}"
        );
        assert!(unique.contains("WHERE external_id IS NOT NULL"), "{unique}");

        let pending = library_files::pending_source_release_index().to_string(PostgresQueryBuilder);
        assert!(
            pending.contains("idx_library_files_pending_source_release"),
            "{pending}"
        );
        assert!(
            pending.contains("delete_source_after_processing AND source_released_at IS NULL"),
            "{pending}"
        );

        let sha256 = library_files::group_sha256_index().to_string(PostgresQueryBuilder);
        assert!(
            sha256.contains("idx_library_files_group_sha256"),
            "{sha256}"
        );
    }

    #[test]
    fn declarations_cover_library_file_document_ddl() {
        let documents = rendered(library_file_documents::schema());
        assert!(
            documents.contains("\"context69\".\"library_file_documents\""),
            "{documents}"
        );
        assert!(
            documents.contains("PRIMARY KEY (\"file_id\", \"document_id\")"),
            "{documents}"
        );
        assert!(
            documents.contains(
                "CONSTRAINT \"library_file_documents_file_id_section_key_key\" UNIQUE (\"file_id\", \"section_key\")"
            ),
            "{documents}"
        );
        assert!(!documents.contains("CREATE UNIQUE INDEX"), "{documents}");
        assert!(
            documents.contains("\"section_metadata_json\" jsonb NOT NULL DEFAULT ('{}'::jsonb)"),
            "{documents}"
        );
        assert!(
            documents.contains("\"sort_order\" integer NOT NULL DEFAULT 0"),
            "{documents}"
        );

        let group = library_file_documents::group_id_index().to_string(PostgresQueryBuilder);
        assert!(
            group.contains("idx_library_file_documents_group_id"),
            "{group}"
        );
    }

    #[test]
    fn declarations_cover_library_storage_object_ddl() {
        let objects = rendered(library_storage_objects::schema());
        assert!(
            objects.contains("\"context69\".\"library_storage_objects\""),
            "{objects}"
        );
        assert!(
            objects.contains("\"id\" uuid NOT NULL PRIMARY KEY"),
            "{objects}"
        );
        assert!(objects.contains("CHECK (length(sha256) = 64)"), "{objects}");
        assert!(
            objects.contains("CHECK (storage_backend IN ('local', 's3'))"),
            "{objects}"
        );
        assert!(
            objects.contains(
                "CONSTRAINT \"library_storage_objects_group_id_sha256_key\" UNIQUE (\"group_id\", \"sha256\")"
            ),
            "{objects}"
        );
        assert!(!objects.contains("CREATE UNIQUE INDEX"), "{objects}");

        let lease = library_storage_objects::staging_lease_index().to_string(PostgresQueryBuilder);
        assert!(
            lease.contains("idx_library_storage_objects_staging_lease"),
            "{lease}"
        );
        assert!(
            lease.contains("WHERE staging_lease_until IS NOT NULL"),
            "{lease}"
        );
    }

    #[test]
    fn declarations_cover_library_legacy_cleanup_ddl() {
        let cleanup = rendered(library_legacy_object_cleanup::schema());
        assert!(
            cleanup.contains("\"context69\".\"library_legacy_object_cleanup\""),
            "{cleanup}"
        );
        assert!(
            cleanup.contains("\"id\" bigint NOT NULL PRIMARY KEY"),
            "{cleanup}"
        );
        assert!(
            cleanup.contains("GENERATED ALWAYS AS IDENTITY"),
            "{cleanup}"
        );
        assert!(
            cleanup.contains("CHECK (deleted_at IS NULL OR deleted_at >= migrated_at)"),
            "{cleanup}"
        );
        assert!(
            cleanup.contains(
                "CHECK (old_storage_backend IS NULL OR btrim(old_storage_backend) <> '')"
            ),
            "{cleanup}"
        );
        assert!(
            cleanup.contains(
                "CONSTRAINT \"library_legacy_object_cleanup_file_id_old_key_key\" UNIQUE (\"file_id\", \"old_key\")"
            ),
            "{cleanup}"
        );
        assert!(!cleanup.contains("CREATE UNIQUE INDEX"), "{cleanup}");

        let eligible =
            library_legacy_object_cleanup::eligible_index().to_string(PostgresQueryBuilder);
        assert!(
            eligible.contains("idx_library_legacy_object_cleanup_eligible"),
            "{eligible}"
        );
        assert!(eligible.contains("WHERE deleted_at IS NULL"), "{eligible}");
    }

    #[test]
    fn declarations_cover_library_dependency_gate_ddl() {
        let gates = rendered(library_dependency_gates::schema());
        assert!(
            gates.contains("\"context69\".\"library_dependency_gates\""),
            "{gates}"
        );
        assert!(
            gates.contains("\"dependency_key\" text NOT NULL PRIMARY KEY"),
            "{gates}"
        );
        assert!(
            gates.contains("\"state\" text NOT NULL DEFAULT 'closed'"),
            "{gates}"
        );
        assert!(
            gates.contains("CHECK (state IN ('closed', 'open', 'half_open'))"),
            "{gates}"
        );
        assert!(gates.contains("CHECK (failure_count >= 0)"), "{gates}");

        let probe = library_dependency_gates::probe_index().to_string(PostgresQueryBuilder);
        assert!(
            probe.contains("idx_library_dependency_gates_probe"),
            "{probe}"
        );
    }

    #[test]
    fn declarations_cover_personal_access_token_ddl() {
        let tokens = rendered(personal_access_tokens::schema());
        assert!(tokens.contains("\"context69\".\"personal_access_tokens\""));
        assert!(tokens.contains("\"scopes\" text[] NOT NULL"), "{tokens}");
        assert!(tokens.contains("UNIQUE"), "{tokens}");
        assert!(
            tokens.contains("CHECK (cardinality(scopes) > 0)"),
            "{tokens}"
        );

        let user = personal_access_tokens::user_index().to_string(PostgresQueryBuilder);
        assert!(
            user.contains("idx_personal_access_tokens_user_id"),
            "{user}"
        );
        assert!(user.contains("\"created_at\" DESC"), "{user}");

        let active = personal_access_tokens::active_index().to_string(PostgresQueryBuilder);
        assert!(
            active.contains("idx_personal_access_tokens_active"),
            "{active}"
        );
    }
}
