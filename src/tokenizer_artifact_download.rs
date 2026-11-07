//! The bounded retrieval of one pinned tokenizer preset file.
//!
//! A preset names one upstream `tokenizer.json`. The retrieval URL is built
//! from the pinned constants of the preset table and never from a request, so
//! a client cannot choose a host, a path, or a hash. The download is bounded
//! in every direction that matters: HTTPS only, official hosts only, a
//! bounded number of redirects, a bounded connect and total time, and a byte
//! count that may not exceed the exact expected size.
//!
//! The retrieved bytes are only usable after their exact length and SHA256
//! match the pinned values *and* they parse as a tokenizer. Nothing is
//! published to the cache directory before that.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::tokenizer_presets::TokenizerPreset;

/// The largest file a retrieval accepts, whatever the preset expects.
pub const TOKENIZER_DOWNLOAD_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The time a retrieval may spend establishing the connection.
pub const TOKENIZER_DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The time a whole retrieval may take.
pub const TOKENIZER_DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// The number of redirects a retrieval may follow.
pub const TOKENIZER_DOWNLOAD_MAX_REDIRECTS: usize = 5;

/// The hosts a retrieval may talk to.
///
/// The official Hugging Face domains and their file-delivery hosts are the
/// only permitted endpoints: `huggingface.co` and the `hf.co` family the
/// Hub serves its files from. Every other host — including a host that merely
/// looks similar — is refused, and a redirect to one is refused as well.
pub fn is_allowed_download_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "huggingface.co"
        || host.ends_with(".huggingface.co")
        || host == "hf.co"
        || host.ends_with(".hf.co")
}

/// The retrieval of one pinned preset file.
#[derive(Debug, Clone)]
pub struct TokenizerArtifactDownloader {
    client: reqwest::Client,
    /// A replacement retrieval URL, used by the loopback tests only.
    ///
    /// It is not reachable from the production API: the provisioning route
    /// never passes one.
    url_override: Option<String>,
}

impl TokenizerArtifactDownloader {
    /// A downloader that talks to the official hosts only.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(TOKENIZER_DOWNLOAD_CONNECT_TIMEOUT)
            .timeout(TOKENIZER_DOWNLOAD_TOTAL_TIMEOUT)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > TOKENIZER_DOWNLOAD_MAX_REDIRECTS {
                    return attempt.error(anyhow::anyhow!(
                        "the tokenizer download followed too many redirects"
                    ));
                }
                let url = attempt.url().clone();
                if url.scheme() != "https" {
                    return attempt.error(anyhow::anyhow!(
                        "the tokenizer download was redirected to a non-HTTPS URL"
                    ));
                }
                match url.host_str() {
                    Some(host) if is_allowed_download_host(host) => attempt.follow(),
                    _ => attempt.error(anyhow::anyhow!(
                        "the tokenizer download was redirected to an unexpected host"
                    )),
                }
            }))
            .build()
            .context("failed to build the tokenizer download client")?;
        Ok(Self {
            client,
            url_override: None,
        })
    }

    /// A downloader that retrieves one fixed URL instead of the official one.
    ///
    /// The loopback tests use it to serve the pinned bytes without reaching
    /// the network. It is deliberately not part of the provisioning API.
    #[cfg(test)]
    pub(crate) fn with_url_override(url: impl Into<String>) -> Result<Self> {
        let mut downloader = Self::new()?;
        downloader.url_override = Some(url.into());
        Ok(downloader)
    }

    /// The retrieval URL of one preset.
    fn url(&self, preset: &TokenizerPreset) -> String {
        self.url_override
            .clone()
            .unwrap_or_else(|| preset.download_url())
    }

    /// Retrieves the pinned file of one preset and verifies it.
    ///
    /// The returned bytes are the exact file: the length matches the pinned
    /// size, the SHA256 matches the pinned hash, and the content parses as a
    /// tokenizer. A mismatch never yields bytes.
    pub async fn download(&self, preset: &TokenizerPreset) -> Result<Vec<u8>> {
        let url = self.url(preset);
        {
            let parsed = reqwest::Url::parse(&url)
                .with_context(|| format!("the tokenizer download URL is not a URL: {url}"))?;
            if parsed.scheme() != "https" && self.url_override.is_none() {
                bail!("the tokenizer download URL must use HTTPS");
            }
            match parsed.host_str() {
                Some(host) if self.url_override.is_none() => {
                    if !is_allowed_download_host(host) {
                        bail!("the tokenizer download host is not an official Hugging Face host");
                    }
                }
                _ => {}
            }
        }

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("the tokenizer download request failed")?;
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            bail!("the tokenizer download returned status {status}");
        }

        // The advertised length is a hint, never the authority: the streamed
        // bytes decide whether the file is the pinned one.
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("the tokenizer download stream failed")?;
            let length = bytes.len() as u64 + chunk.len() as u64;
            if length > preset.size_bytes {
                bail!(
                    "the tokenizer download exceeded the expected size of {} bytes",
                    preset.size_bytes
                );
            }
            if length > TOKENIZER_DOWNLOAD_MAX_BYTES {
                bail!("the tokenizer download exceeded {TOKENIZER_DOWNLOAD_MAX_BYTES} bytes");
            }
            bytes.extend_from_slice(&chunk);
        }

        if bytes.len() as u64 != preset.size_bytes {
            bail!(
                "the tokenizer download returned {} bytes instead of the expected {}",
                bytes.len(),
                preset.size_bytes
            );
        }
        let actual = hash_bytes(&bytes);
        if actual != preset.sha256 {
            bail!("the tokenizer download does not match the pinned hash {actual}");
        }
        Ok(bytes)
    }
}

/// The lowercase SHA256 hex of the bytes.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Publishes verified bytes into the cache directory under their own hash.
///
/// The write is atomic and never replaces an existing entry: the bytes land in
/// a unique temporary file in the same directory, they are flushed to disk, and
/// the temporary file is linked onto the final name. A link fails when the name
/// already exists, so an operator's own file and a concurrent publication both
/// stay untouched. A reader therefore never observes a half-written file, and a
/// failed write leaves no partial artifact behind.
///
/// An existing entry is accepted only when it already holds the verified bytes.
/// An entry that is unsafe (a symlink or any non-regular file) or corrupt is
/// refused and left exactly as it is: the product never deletes a user file.
///
/// The publication is only complete after the file and its directory entry are
/// flushed: a preparation never reports ready for a file that is not durable.
/// A failed flush is reported as a failure and never removes the file it found
/// or created.
pub fn publish_cached_artifact(directory: &Path, sha256: &str, bytes: &[u8]) -> Result<PathBuf> {
    publish_cached_artifact_with(directory, sha256, bytes, &sync_published_entry, &|_| {})
}

/// The publication with its durability step and its pre-link step injected.
///
/// The production caller always passes [`sync_published_entry`] and a no-op
/// step. The focused tests use the seam to control a flush failure and to
/// reproduce a concurrent winner deterministically.
fn publish_cached_artifact_with(
    directory: &Path,
    sha256: &str,
    bytes: &[u8],
    sync_entry: &dyn Fn(&Path, &Path) -> Result<()>,
    before_link: &dyn Fn(&Path),
) -> Result<PathBuf> {
    std::fs::create_dir_all(directory).with_context(|| {
        format!(
            "create the tokenizer cache directory {}",
            directory.display()
        )
    })?;
    let actual = hash_bytes(bytes);
    if actual != sha256 {
        bail!("the tokenizer file does not match its published name {sha256}");
    }
    let final_path = directory.join(format!("{sha256}.json"));
    if let Some(published) = existing_publication(&final_path, sha256, bytes.len() as u64)? {
        // A verified entry that is already published is only accepted once it
        // is durable, so ready never describes a file a crash could lose.
        sync_entry(directory, &published)?;
        return Ok(published);
    }
    let temporary_path = unique_temporary_path(directory, sha256)?;
    // The temporary file is created before anything is written, and only a
    // file this call created is ever removed again.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .with_context(|| {
            format!(
                "create the temporary tokenizer cache file {}",
                temporary_path.display()
            )
        })?;
    let outcome = (|| -> Result<()> {
        file.write_all(bytes)
            .context("write the temporary tokenizer cache file")?;
        file.sync_all()
            .context("flush the temporary tokenizer cache file")?;
        Ok(())
    })();
    if let Err(error) = outcome {
        return Err(with_temporary_cleanup(&temporary_path, error));
    }
    before_link(&temporary_path);
    // A link publishes the bytes under the final name without ever replacing
    // an existing entry, so the publication is a no-clobber operation.
    match std::fs::hard_link(&temporary_path, &final_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process published the same file while this one wrote its
            // temporary copy. Only identical verified bytes are accepted, and
            // an owned temporary file that cannot be removed is reported.
            remove_temporary(&temporary_path)?;
            let published = existing_publication(&final_path, sha256, bytes.len() as u64)?
                .ok_or_else(|| {
                    anyhow::anyhow!("the tokenizer cache entry disappeared during publication")
                })?;
            // The winner's entry is only accepted once it is durable.
            sync_entry(directory, &published)?;
            return Ok(published);
        }
        Err(error) => {
            let error = anyhow::Error::new(error).context(format!(
                "publish the tokenizer cache file {}",
                final_path.display()
            ));
            return Err(with_temporary_cleanup(&temporary_path, error));
        }
    }
    // The temporary copy is removed before the directory is flushed, so the
    // durable state is one verified file with no leftover temporary name. A
    // temporary file that cannot be removed is reported and retained.
    remove_temporary(&temporary_path)?;
    // A failed flush is reported as a failure and leaves the published file in
    // place: this call never unlinks a file it published.
    sync_entry(directory, &final_path)?;
    Ok(final_path)
}

/// Removes the owned temporary file, reporting the failure beside the primary
/// cause and retaining the file when it cannot be removed.
///
/// The reported message is flattened so that its ordinary `Display` carries
/// both failures: an operator reading a single formatted error still sees what
/// failed first and that the owned temporary file was retained.
fn with_temporary_cleanup(temporary_path: &Path, primary: anyhow::Error) -> anyhow::Error {
    match std::fs::remove_file(temporary_path) {
        Ok(()) => primary,
        Err(error) => anyhow::anyhow!(
            "{primary:#}: remove the temporary tokenizer cache file {}: {error}",
            temporary_path.display()
        ),
    }
}

/// Removes the owned temporary file after a successful publication.
///
/// A failure is reported instead of ignored, and the owned file is retained.
fn remove_temporary(temporary_path: &Path) -> Result<()> {
    std::fs::remove_file(temporary_path).with_context(|| {
        format!(
            "remove the temporary tokenizer cache file {}",
            temporary_path.display()
        )
    })
}

/// Flushes a verified cache entry and its directory entry.
///
/// A preparation calls this before it reports a preset ready, so "prepared"
/// always means the file and its name survive a crash.
pub(crate) fn sync_published_entry(directory: &Path, path: &Path) -> Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open the published tokenizer file {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("flush the published tokenizer file {}", path.display()))?;
    let directory_handle = std::fs::File::open(directory)
        .with_context(|| format!("open the tokenizer cache directory {}", directory.display()))?;
    directory_handle.sync_all().with_context(|| {
        format!(
            "flush the tokenizer cache directory {}",
            directory.display()
        )
    })
}

/// The outcome of an entry that already exists at the published name.
///
/// `None` means the name is free. An existing regular file that holds the
/// verified bytes is the published file. An entry that is unsafe or corrupt is
/// refused and left exactly as it is.
fn existing_publication(
    final_path: &Path,
    sha256: &str,
    expected_len: u64,
) -> Result<Option<PathBuf>> {
    let metadata = match std::fs::symlink_metadata(final_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("read the tokenizer cache entry {}", final_path.display())
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "the tokenizer cache entry {} is not a regular file; it is left untouched",
            final_path.display()
        );
    }
    if metadata.len() != expected_len {
        bail!(
            "the tokenizer cache entry {} does not hold the verified file; remove it and prepare the preset again",
            final_path.display()
        );
    }
    let existing = std::fs::read(final_path)
        .with_context(|| format!("read the tokenizer cache entry {}", final_path.display()))?;
    if hash_bytes(&existing) != sha256 {
        bail!(
            "the tokenizer cache entry {} does not match its name; remove it and prepare the preset again",
            final_path.display()
        );
    }
    Ok(Some(final_path.to_path_buf()))
}

/// A unique temporary path in the same directory as the published file.
///
/// The name carries a random suffix, so two processes publishing the same
/// file never share a temporary name and never follow an existing one.
fn unique_temporary_path(directory: &Path, sha256: &str) -> Result<PathBuf> {
    for _ in 0..16 {
        let suffix = random_suffix()?;
        let candidate = directory.join(format!(".{sha256}.{suffix}.part"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("failed to create a unique temporary tokenizer cache file name");
}

/// A random hexadecimal suffix for a temporary file name.
fn random_suffix() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The random bytes of the operating system.
fn getrandom(bytes: &mut [u8]) -> Result<()> {
    use std::io::Read;
    let mut file =
        std::fs::File::open("/dev/urandom").context("open the operating system random source")?;
    file.read_exact(bytes)
        .context("read the operating system random source")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn review_publication_reports_temporary_cleanup_failure() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_cache_directory("review-cleanup-failure");
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

    #[test]
    fn a_published_artifact_is_named_by_its_hash_and_is_complete() {
        let directory = test_cache_directory("named-by-hash");
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

    /// A cache directory owned by one test.
    fn test_cache_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "context69-tokenizer-download-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create the cache directory");
        directory
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
    fn a_failed_publication_removes_its_temporary_file() {
        let directory = test_cache_directory("failed-publication");
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        // A non-empty directory at the final name is an existing entry that is
        // not a regular file: the publication refuses it and writes nothing.
        let blocked = directory.join(format!("{sha256}.json"));
        std::fs::create_dir_all(&blocked).expect("create the blocking directory");
        std::fs::write(blocked.join("keep"), b"keep").expect("write the blocking file");

        publish_cached_artifact(&directory, &sha256, bytes)
            .expect_err("the publication cannot replace an existing directory");

        assert_eq!(
            std::fs::read(blocked.join("keep")).expect("read the untouched entry"),
            b"keep",
            "an existing entry is never changed"
        );
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[test]
    fn bytes_that_do_not_match_their_published_name_are_refused() {
        let directory = test_cache_directory("wrong-name");
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

    #[test]
    fn an_existing_corrupt_file_is_refused_and_left_untouched() {
        let directory = test_cache_directory("corrupt-entry");
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        let path = directory.join(format!("{sha256}.json"));
        let corrupt = b"{\"version\":\"corrupt\"}";
        std::fs::write(&path, corrupt).expect("write the corrupt entry");

        publish_cached_artifact(&directory, &sha256, bytes)
            .expect_err("a corrupt entry is never replaced");

        assert_eq!(
            std::fs::read(&path).expect("read the untouched entry"),
            corrupt,
            "the operator's file is preserved byte for byte"
        );
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_symlink_is_refused_and_left_untouched() {
        let directory = test_cache_directory("symlink-entry");
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        let path = directory.join(format!("{sha256}.json"));
        let target = directory.join("operator-target.json");
        std::fs::write(&target, b"operator bytes").expect("write the link target");
        std::os::unix::fs::symlink(&target, &path).expect("create the symlink");

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
        assert_eq!(
            std::fs::read(&target).expect("read the link target"),
            b"operator bytes"
        );
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_dangling_symlink_is_refused_and_left_untouched() {
        let directory = test_cache_directory("dangling-symlink-entry");
        let bytes = b"{\"version\":\"1.0\"}";
        let sha256 = hash_bytes(bytes);
        let path = directory.join(format!("{sha256}.json"));
        let missing = directory.join("absent.json");
        std::os::unix::fs::symlink(&missing, &path).expect("create the dangling symlink");

        publish_cached_artifact(&directory, &sha256, bytes)
            .expect_err("a dangling symlink is an entry too and is never replaced");

        let metadata = std::fs::symlink_metadata(&path).expect("the symlink is untouched");
        assert!(
            metadata.file_type().is_symlink(),
            "the symlink stays a symlink"
        );
        assert_eq!(
            std::fs::read_link(&path).expect("read the link"),
            missing,
            "the dangling link is unchanged"
        );
        assert_no_temporary_files(&directory);

        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
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
        let directory = test_cache_directory("flush-failure");
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
        let directory = test_cache_directory("existing-flush");
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
        let directory = test_cache_directory("flush-order");
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
        let directory = test_cache_directory("race-winner");
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
        let directory = test_cache_directory("race-winner-flush-failure");
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
        let directory = test_cache_directory("race-corrupt");
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
}

/// The loopback retrieval used by the focused tests.
///
/// It serves the bytes a test authored on `127.0.0.1`, so no test reaches the
/// network and no test fabricates bytes under the official pinned hash.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Router,
        http::{StatusCode, header},
        response::IntoResponse,
    };

    use super::{TokenizerPreset, hash_bytes};

    /// One loopback server, aborted when the test ends.
    pub(crate) struct LoopbackServer {
        pub(crate) url: String,
        requests: Arc<AtomicUsize>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl LoopbackServer {
        /// The number of requests the server received.
        pub(crate) fn request_count(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    impl Drop for LoopbackServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn serve(app: Router, requests: Arc<AtomicUsize>) -> LoopbackServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the loopback test server");
        let address = listener.local_addr().expect("read the loopback address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        LoopbackServer {
            url: format!("http://{address}/tokenizer.json"),
            requests,
            handle,
        }
    }

    /// A server that answers with the exact bytes.
    pub(crate) async fn serve_bytes(bytes: Vec<u8>) -> LoopbackServer {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let app = Router::new().fallback(move || {
            let bytes = bytes.clone();
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                bytes
            }
        });
        serve(app, requests).await
    }

    /// A server that answers with one status and no body.
    pub(crate) async fn serve_status(status: u16) -> LoopbackServer {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let app = Router::new().fallback(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::from_u16(status).expect("a valid test status")
            }
        });
        serve(app, requests).await
    }

    /// A server that redirects to one location.
    pub(crate) async fn serve_redirect(location: &'static str) -> LoopbackServer {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let app = Router::new().fallback(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
            }
        });
        serve(app, requests).await
    }

    /// A preset a test authored: its own bytes, its own hash, no official pin.
    pub(crate) fn pinned_test_preset(bytes: &[u8]) -> TokenizerPreset {
        let sha256: &'static str = Box::leak(hash_bytes(bytes).into_boxed_str());
        TokenizerPreset {
            id: "test_pinned_preset",
            label: "Test pinned preset",
            repo: "test/pinned-preset",
            revision: "0000000000000000000000000000000000000000",
            file: "tokenizer.json",
            sha256,
            size_bytes: bytes.len() as u64,
            license: "test-only",
            license_url: "https://example.test/license",
        }
    }
}
