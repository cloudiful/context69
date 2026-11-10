-- Code-source target/profile editing and safe failure detail (issue #28 P4).
--
-- The obsolete mutually-exclusive `refresh_policy` knob is removed from the
-- code-source contract: periodic interval and webhook-enabled are the complete
-- refresh controls and are independent of each other.
--
-- `last_error_code` carries only a bounded stable classification of a terminal
-- index failure, so the source response can explain a failed run without
-- exposing a provider message, response body, or secret-bearing detail.

ALTER TABLE context69.git_repository_sources
    DROP COLUMN refresh_policy,
    ADD COLUMN last_error_code TEXT
        CHECK (
            last_error_code IS NULL
            OR (
                char_length(last_error_code) BETWEEN 1 AND 64
                AND last_error_code ~ '^[a-z0-9_]+$'
            )
        );
