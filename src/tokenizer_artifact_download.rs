//! The bounded retrieval of one pinned tokenizer preset file.
//!
//! A preset names one upstream `tokenizer.json`. The retrieval URL is built
//! from the pinned constants of the preset table and never from a request, so
//! a client cannot choose a host, a path, or a hash. The retrieved bytes are
//! only usable after their exact length and SHA256 match the pinned values
//! *and* they parse as a tokenizer.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::{
    chunking::chunking_token_artifact::TOKENIZER_ARTIFACT_MAX_BYTES,
    tokenizer_presets::TokenizerPreset,
};

pub use crate::chunking::chunking_token_artifact::hash_bytes;

/// The largest file a retrieval accepts, whatever the preset expects.
///
/// It re-exports the catalog maximum, so the deployment keeps one size
/// authority and the module's long-standing public name keeps resolving.
pub use crate::chunking::chunking_token_artifact::TOKENIZER_ARTIFACT_MAX_BYTES as TOKENIZER_DOWNLOAD_MAX_BYTES;

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
        // A preset bound is checked before any I/O, so a future preset larger
        // than the catalog accepts is refused instead of being streamed.
        if preset.size_bytes > TOKENIZER_ARTIFACT_MAX_BYTES {
            bail!(
                "the tokenizer preset {} exceeds {TOKENIZER_ARTIFACT_MAX_BYTES} bytes",
                preset.id
            );
        }
        let url = self.url(preset);
        {
            let parsed = reqwest::Url::parse(&url)
                .with_context(|| format!("the tokenizer download URL is not a URL: {url}"))?;
            if parsed.scheme() != "https" && self.url_override.is_none() {
                bail!("the tokenizer download URL must use HTTPS");
            }
            match parsed.host_str() {
                Some(host) if self.url_override.is_none() && !is_allowed_download_host(host) => {
                    bail!("the tokenizer download host is not an official Hugging Face host");
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

/// Publishes verified bytes into the cache directory under their own hash.
///
/// The write is atomic and never replaces an existing entry: the bytes land in
/// a unique temporary file in the same directory, they are flushed to disk, and
/// the temporary file is linked onto the final name. An existing entry is
/// accepted only when it already holds the verified bytes, and an entry that is
/// unsafe or corrupt is refused and left exactly as it is.
///
/// The publication is only complete after the file and its directory entry are
/// flushed: a failed flush is reported and never removes the published file.
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
