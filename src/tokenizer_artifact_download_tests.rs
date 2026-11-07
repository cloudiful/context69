//! Cases for the bounded retrieval and the no-clobber, durable publication.
//!
//! The loopback transport lives in the module's own `test_support`, so every
//! case serves bytes it authored itself and never reaches the network.

use super::*;

#[cfg(unix)]
#[test]
fn review_publication_reports_temporary_cleanup_failure() {
    use std::os::unix::fs::PermissionsExt;

    let directory = test_support::test_directory("review-cleanup-failure");
    let bytes = b"test tokenizer bytes";
    let sha256 = hash_bytes(bytes);
    let dir = directory.clone();
    let before_link = move |_: &Path| {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))
            .expect("deny directory writes");
    };
    let result = publish_cached_artifact_with(
        &directory,
        &sha256,
        bytes,
        &sync_published_entry,
        &before_link,
    );
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("restore directory writes");
    let leftovers = std::fs::read_dir(&directory)
        .expect("read directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
        .count();
    let failure = result.expect_err("publication must fail");
    let error = format!("{failure:#}");
    let logged_error = format!("{failure}");
    std::fs::remove_dir_all(&directory).expect("remove own cache");
    assert_eq!(
        leftovers, 1,
        "the controlled cleanup failure leaves one temp"
    );
    assert!(
        error.contains("remove the temporary"),
        "the returned error must report the cleanup failure: {error}"
    );
    assert!(
        logged_error.contains("publish the tokenizer cache file"),
        "the service logs Display formatting and must retain the primary error: {logged_error}"
    );
}

#[test]
fn only_official_hugging_face_hosts_are_allowed() {
    for host in [
        "huggingface.co",
        "cdn-lfs.huggingface.co",
        "hf.co",
        "cas-server.xethub.hf.co",
        "cas-bridge.xethub-eu.hf.co",
        "transfer.xethub.hf.co",
    ] {
        assert!(is_allowed_download_host(host), "{host} must be allowed");
    }
    for host in [
        "example.com",
        "huggingface.co.evil.test",
        "evil-huggingface.co",
        "hf.co.evil.test",
        "localhost",
        "127.0.0.1",
    ] {
        assert!(!is_allowed_download_host(host), "{host} must be refused");
    }
}

#[tokio::test]
async fn a_verified_retrieval_returns_the_exact_bytes() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_bytes(bytes.clone()).await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    let retrieved = downloader
        .download(&preset)
        .await
        .expect("retrieve the preset");

    assert_eq!(retrieved, bytes);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_retrieval_of_a_different_size_is_refused() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_bytes(bytes[..bytes.len() - 4].to_vec()).await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    let error = downloader
        .download(&preset)
        .await
        .expect_err("a short file is refused");

    assert!(error.to_string().contains("bytes instead of the expected"));
}

#[tokio::test]
async fn a_retrieval_of_a_different_hash_is_refused() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    // Same length, different content: only the hash can tell them apart.
    let mut other = bytes.clone();
    let last = other.len() - 2;
    other[last] = if other[last] == b' ' { b'!' } else { b' ' };
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_bytes(other).await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    let error = downloader
        .download(&preset)
        .await
        .expect_err("a different file is refused");

    assert!(error.to_string().contains("does not match the pinned hash"));
}

#[tokio::test]
async fn a_retrieval_that_exceeds_the_expected_size_is_refused() {
    // The former public size name still resolves and names the one authority.
    let maximum: u64 = crate::tokenizer_artifact_download::TOKENIZER_DOWNLOAD_MAX_BYTES;
    assert_eq!(
        maximum,
        crate::chunking::chunking_token_artifact::TOKENIZER_ARTIFACT_MAX_BYTES
    );

    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let mut longer = bytes.clone();
    longer.extend_from_slice(b"excess bytes");
    let server = test_support::serve_bytes(longer).await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    let error = downloader
        .download(&preset)
        .await
        .expect_err("an oversized file is refused");

    assert!(error.to_string().contains("exceeded the expected size"));
}

#[tokio::test]
async fn a_non_200_response_is_refused() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_status(404).await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    let error = downloader
        .download(&preset)
        .await
        .expect_err("a non-200 response is refused");

    assert!(error.to_string().contains("status 404"));
}

#[tokio::test]
async fn a_redirect_to_an_unexpected_host_is_refused() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_redirect("https://example.test/tokenizer.json").await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    downloader
        .download(&preset)
        .await
        .expect_err("a redirect to an unexpected host is refused");
}

#[tokio::test]
async fn a_redirect_downgrade_to_http_is_refused() {
    let bytes = crate::chunking::chunking_token_artifact::test_tokenizer_json(false);
    let preset = test_support::pinned_test_preset(&bytes);
    let server = test_support::serve_redirect("http://huggingface.co/tokenizer.json").await;
    let downloader = TokenizerArtifactDownloader::with_url_override(&server.url)
        .expect("build the loopback downloader");

    downloader
        .download(&preset)
        .await
        .expect_err("a plaintext redirect is refused");
}

#[test]
fn a_published_artifact_is_named_by_its_hash_and_is_complete() {
    let directory = test_support::test_directory("named-by-hash");
    let bytes = b"{\"version\":\"1.0\"}";
    let sha256 = hash_bytes(bytes);

    let published = publish_cached_artifact(&directory, &sha256, bytes).expect("publish");
    assert_eq!(published, directory.join(format!("{sha256}.json")));
    assert_eq!(
        std::fs::read(&published).expect("read the published file"),
        bytes
    );

    // A second publication of the same verified bytes is idempotent: the
    // published file stays exactly as it is and no temporary file remains.
    #[cfg(unix)]
    let inode_before = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&published)
            .expect("stat the published file")
            .ino()
    };
    assert_eq!(
        publish_cached_artifact(&directory, &sha256, bytes).expect("publish again"),
        published
    );
    assert_eq!(
        std::fs::read(&published).expect("read the published file"),
        bytes
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&published)
                .expect("stat the published file")
                .ino(),
            inode_before,
            "the verified file is not replaced"
        );
    }
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn bytes_that_do_not_match_their_published_name_are_refused() {
    let directory = test_support::test_directory("wrong-name");
    let bytes = b"{\"version\":\"1.0\"}";

    publish_cached_artifact(&directory, &hash_bytes(b"other bytes"), bytes)
        .expect_err("the bytes must match the name they are published under");

    assert_no_temporary_files(&directory);
    assert_eq!(
        std::fs::read_dir(&directory)
            .expect("read the cache directory")
            .filter_map(|entry| entry.ok())
            .count(),
        0,
        "nothing is published"
    );

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

/// Every unusable entry shape is refused and left exactly as it was.
#[test]
fn an_unusable_entry_is_refused_and_left_untouched() {
    for case in ["directory", "corrupt"] {
        let directory = test_support::test_directory(&format!("unusable-entry-{case}"));
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        let path = directory.join(format!("{sha256}.json"));
        match case {
            "directory" => {
                // A non-empty directory at the final name is an entry that is
                // not a regular file.
                std::fs::create_dir_all(&path).expect("create the blocking directory");
                std::fs::write(path.join("keep"), b"keep").expect("write the blocking file");
            }
            "corrupt" => {
                std::fs::write(&path, b"{\"version\":\"corrupt\"}").expect("write the entry");
            }
            other => unreachable!("unknown unusable-entry case {other}"),
        }

        publish_cached_artifact(&directory, &sha256, bytes)
            .expect_err("an unusable entry is never replaced");

        match case {
            "directory" => assert_eq!(
                std::fs::read(path.join("keep")).expect("read the untouched entry"),
                b"keep",
                "an existing entry is never changed"
            ),
            "corrupt" => assert_eq!(
                std::fs::read(&path).expect("read the untouched entry"),
                b"{\"version\":\"corrupt\"}",
                "the operator's file is preserved byte for byte"
            ),
            other => unreachable!("unknown unusable-entry case {other}"),
        }
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }
}

/// A symlink entry is refused without following or writing through it.
#[cfg(unix)]
#[test]
fn an_unsafe_symlink_entry_is_refused_and_left_untouched() {
    for case in ["symlink", "dangling-symlink"] {
        let directory = test_support::test_directory(&format!("unsafe-entry-{case}"));
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        let path = directory.join(format!("{sha256}.json"));
        let target = match case {
            "symlink" => {
                let target = directory.join("operator-target.json");
                std::fs::write(&target, b"operator bytes").expect("write the link target");
                std::os::unix::fs::symlink(&target, &path).expect("create the symlink");
                target
            }
            "dangling-symlink" => {
                let missing = directory.join("absent.json");
                std::os::unix::fs::symlink(&missing, &path).expect("create the dangling symlink");
                missing
            }
            other => unreachable!("unknown symlink-entry case {other}"),
        };

        publish_cached_artifact(&directory, &sha256, bytes)
            .expect_err("a symlink entry is never followed or replaced");

        let metadata = std::fs::symlink_metadata(&path).expect("the symlink is untouched");
        assert!(
            metadata.file_type().is_symlink(),
            "the symlink stays a symlink"
        );
        assert_eq!(
            std::fs::read_link(&path).expect("read the link"),
            target,
            "the link target is unchanged"
        );
        if case == "symlink" {
            assert_eq!(
                std::fs::read(&target).expect("read the link target"),
                b"operator bytes",
                "the link target bytes are unchanged"
            );
        }
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }
}

/// The owned temporary files of one test.
fn assert_no_temporary_files(directory: &Path) {
    assert!(
        !has_temporary_files(directory),
        "no temporary file may survive"
    );
}

/// Whether the directory still holds an owned temporary file.
fn has_temporary_files(directory: &Path) -> bool {
    std::fs::read_dir(directory)
        .expect("read the cache directory")
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".part"))
        })
}

/// A uniqueness scope for the controlled publication tests.
fn publication_bytes(name: &str) -> Vec<u8> {
    format!("{{\"version\":\"1.0\",\"name\":\"{name}\"}}").into_bytes()
}

#[test]
fn a_directory_flush_failure_keeps_the_published_file() {
    let directory = test_support::test_directory("flush-failure");
    let bytes = publication_bytes("flush-failure");
    let sha256 = hash_bytes(&bytes);

    publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|_, _| Err(anyhow::anyhow!("the directory flush failed")),
        &|_| {},
    )
    .expect_err("a failed flush is reported");

    let path = directory.join(format!("{sha256}.json"));
    assert_eq!(
        std::fs::read(&path).expect("the published file is kept"),
        bytes,
        "a failed flush never unlinks the published file"
    );
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn a_verified_existing_entry_is_flushed_before_it_is_accepted() {
    let directory = test_support::test_directory("existing-flush");
    let bytes = publication_bytes("existing-flush");
    let sha256 = hash_bytes(&bytes);
    publish_cached_artifact(&directory, &sha256, &bytes).expect("publish");
    let path = directory.join(format!("{sha256}.json"));
    #[cfg(unix)]
    let inode_before = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&path)
            .expect("stat the published file")
            .ino()
    };

    // The verified entry is only accepted once it is durable.
    publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|_, _| Err(anyhow::anyhow!("the directory flush failed")),
        &|_| {},
    )
    .expect_err("a failed flush is reported even for an existing entry");

    assert_eq!(
        std::fs::read(&path).expect("the existing entry is kept"),
        bytes,
        "the existing entry is never removed"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("stat the existing entry")
                .ino(),
            inode_before,
            "the existing entry is never replaced"
        );
    }
    assert_no_temporary_files(&directory);

    let flushes = std::cell::Cell::new(0);
    publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|dir, entry| {
            flushes.set(flushes.get() + 1);
            sync_published_entry(dir, entry)
        },
        &|_| {},
    )
    .expect("the verified entry is accepted after a successful flush");
    assert_eq!(
        flushes.get(),
        1,
        "the accepted entry was flushed exactly once"
    );

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn a_successful_publication_flushes_after_removing_the_temporary_copy() {
    let directory = test_support::test_directory("flush-order");
    let bytes = publication_bytes("flush-order");
    let sha256 = hash_bytes(&bytes);
    let flushes = std::cell::Cell::new(0);
    let saw_temporary = std::cell::Cell::new(false);

    let published = publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|dir, entry| {
            flushes.set(flushes.get() + 1);
            saw_temporary.set(saw_temporary.get() || has_temporary_files(dir));
            sync_published_entry(dir, entry)
        },
        &|_| {},
    )
    .expect("publish");

    assert_eq!(published, directory.join(format!("{sha256}.json")));
    assert_eq!(std::fs::read(&published).expect("read the file"), bytes);
    assert_eq!(flushes.get(), 1, "the publication is flushed exactly once");
    assert!(
        !saw_temporary.get(),
        "the temporary copy is removed before the directory is flushed"
    );
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn a_race_winner_is_accepted_only_after_a_successful_flush() {
    let directory = test_support::test_directory("race-winner");
    let bytes = publication_bytes("race-winner");
    let sha256 = hash_bytes(&bytes);
    let winner_directory = directory.clone();
    let winner_bytes = bytes.clone();
    let winner_sha256 = sha256.clone();
    let flushes = std::cell::Cell::new(0);
    let saw_temporary = std::cell::Cell::new(false);
    // The link step loses a race to another process that publishes the
    // same verified file after this call wrote its temporary copy.
    let before_link = move |_temporary: &Path| {
        std::fs::write(
            winner_directory.join(format!("{winner_sha256}.json")),
            &winner_bytes,
        )
        .expect("publish the winner");
    };

    let published = publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|dir, entry| {
            flushes.set(flushes.get() + 1);
            saw_temporary.set(saw_temporary.get() || has_temporary_files(dir));
            sync_published_entry(dir, entry)
        },
        &before_link,
    )
    .expect("the verified winner is accepted");

    assert_eq!(published, directory.join(format!("{sha256}.json")));
    assert_eq!(std::fs::read(&published).expect("read the winner"), bytes);
    assert_eq!(
        flushes.get(),
        1,
        "the race winner is flushed before success is reported"
    );
    assert!(
        !saw_temporary.get(),
        "the temporary copy is removed before the winner is flushed"
    );
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn a_race_winner_is_kept_when_its_flush_fails() {
    let directory = test_support::test_directory("race-winner-flush-failure");
    let bytes = publication_bytes("race-winner-flush-failure");
    let sha256 = hash_bytes(&bytes);
    let winner_directory = directory.clone();
    let winner_bytes = bytes.clone();
    let winner_sha256 = sha256.clone();
    let before_link = move |_temporary: &Path| {
        std::fs::write(
            winner_directory.join(format!("{winner_sha256}.json")),
            &winner_bytes,
        )
        .expect("publish the winner");
    };

    publish_cached_artifact_with(
        &directory,
        &sha256,
        &bytes,
        &|_, _| Err(anyhow::anyhow!("the directory flush failed")),
        &before_link,
    )
    .expect_err("a failed flush is reported");

    assert_eq!(
        std::fs::read(directory.join(format!("{sha256}.json"))).expect("read the winner"),
        bytes,
        "a failed flush never unlinks the race winner"
    );
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}

#[test]
fn a_race_lost_to_a_corrupt_entry_is_refused_and_left_untouched() {
    let directory = test_support::test_directory("race-corrupt");
    let bytes = publication_bytes("race-corrupt");
    let sha256 = hash_bytes(&bytes);
    let corrupt_directory = directory.clone();
    let corrupt_sha256 = sha256.clone();
    let before_link = move |_temporary: &Path| {
        std::fs::write(
            corrupt_directory.join(format!("{corrupt_sha256}.json")),
            b"{\"version\":\"corrupt\"}",
        )
        .expect("publish the corrupt entry");
    };

    publish_cached_artifact_with(&directory, &sha256, &bytes, &|_, _| Ok(()), &before_link)
        .expect_err("a corrupt entry that won the race is refused");

    assert_eq!(
        std::fs::read(directory.join(format!("{sha256}.json"))).expect("read the entry"),
        b"{\"version\":\"corrupt\"}",
        "the corrupt entry is left exactly as it was"
    );
    assert_no_temporary_files(&directory);

    std::fs::remove_dir_all(&directory).expect("remove the cache directory");
}
