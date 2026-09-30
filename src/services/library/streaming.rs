//! Streamed object transfer used by source materialization.
//!
//! Ownership this module implements (issue 667 Phase 1): original uploaded and
//! downloaded bytes are canonical in the object store under a content-addressed
//! key, PostgreSQL keeps the `library_files`/`library_storage_objects` catalog
//! plus the searchable normalized text, Qdrant stays a derived vector index,
//! and task rows stay workflow metadata and checkpoints. A URL task therefore
//! carries an object reference, never the downloaded bytes: `url_imports`
//! streams a remote body through [`transfer`] into a temporary staging key and
//! copies it to `objects/<group>/<sha256>` only once the digest is known.

use anyhow::{Context, Result};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::domain_errors::DomainError;

use super::dependency_storage::bounded_s3_attempt;

/// A body that yields itself in chunks.
#[async_trait::async_trait]
pub(super) trait ChunkSource: Send {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>>;
}

/// A destination for streamed object bytes.
#[async_trait::async_trait]
pub(super) trait ByteSink: Send {
    async fn write_chunk(&mut self, chunk: Bytes) -> Result<()>;
}

/// Canonical digest and size of a fully streamed body.
#[derive(Debug)]
pub(super) struct StreamedDigest {
    pub sha256: String,
    pub size_bytes: i64,
}

/// Incremental object writer used by streamed source materialization.
///
/// Bytes arrive as remote-response chunks, so the write cannot be retried as a
/// whole: replaying a chunk that already reached the store would duplicate
/// bytes in the object. Every call is therefore bounded exactly once, and a
/// failure aborts the object, which the caller deletes.
pub(super) struct StorageStreamWriter {
    writer: opendal::Writer,
    backend: &'static str,
}

impl StorageStreamWriter {
    pub(super) fn new(writer: opendal::Writer, backend: &'static str) -> Self {
        Self { writer, backend }
    }

    pub(super) async fn close(mut self) -> Result<()> {
        if self.backend == "s3" {
            bounded_s3_attempt("writer_close", || self.writer.close())
                .await
                .map(|_| ())
        } else {
            self.writer
                .close()
                .await
                .map(|_| ())
                .with_context(|| DomainError::internal("failed to finalize streamed stored object"))
        }
    }

    /// Abort the writer, best effort.
    ///
    /// Aborting is not a durability guarantee: the local filesystem backend
    /// writes in place and refuses to abort. Deleting the staging key (which
    /// the materializer does on every failure path) is what actually reclaims
    /// the partial bytes.
    pub(super) async fn abort(&mut self, key: &str) {
        if let Err(error) = self.writer.abort().await {
            warn!(key, %error, "failed to abort streamed stored object write");
        }
    }
}

#[async_trait::async_trait]
impl ByteSink for StorageStreamWriter {
    async fn write_chunk(&mut self, chunk: Bytes) -> Result<()> {
        if self.backend == "s3" {
            bounded_s3_attempt("writer_write", || self.writer.write(chunk.clone()))
                .await
                .map_err(|error| {
                    error.context(DomainError::internal("failed to stream stored object"))
                })
        } else {
            self.writer.write(chunk).await.map_err(|error| {
                anyhow::Error::from(error)
                    .context(DomainError::internal("failed to stream stored object"))
            })
        }
    }
}

/// Canonical digest and size of a body, computed while its chunks stream past.
struct StreamAccumulator {
    hasher: Sha256,
    size: u64,
    max_bytes: usize,
}

impl StreamAccumulator {
    fn new(max_bytes: usize) -> Self {
        Self {
            hasher: Sha256::new(),
            size: 0,
            max_bytes,
        }
    }

    fn accept(&mut self, chunk: &[u8]) -> Result<()> {
        let size = self.size.saturating_add(chunk.len() as u64);
        if size > self.max_bytes as u64 {
            return Err(DomainError::payload_too_large("remote_file_too_large").into());
        }
        self.size = size;
        self.hasher.update(chunk);
        Ok(())
    }

    /// Count one chunk and write it onward.
    ///
    /// The chunk is accounted for before it is written, so the digest covers
    /// exactly the bytes the object store accepted, and a sink failure stops
    /// the transfer with the sink's own error.
    async fn forward(&mut self, sink: &mut dyn ByteSink, chunk: Bytes) -> Result<()> {
        self.accept(&chunk)?;
        sink.write_chunk(chunk).await
    }

    fn finish(self) -> StreamedDigest {
        StreamedDigest {
            sha256: self
                .hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            size_bytes: self.size as i64,
        }
    }
}

/// Consume one remote body into `sink`.
///
/// The size limit is enforced per chunk, so an oversized body fails while it
/// streams instead of after it was buffered.
pub(super) async fn transfer(
    source: &mut dyn ChunkSource,
    sink: &mut dyn ByteSink,
    max_bytes: usize,
) -> Result<StreamedDigest> {
    let mut accumulator = StreamAccumulator::new(max_bytes);
    while let Some(chunk) = source.next_chunk().await? {
        accumulator.forward(sink, chunk).await?;
    }
    Ok(accumulator.finish())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use bytes::Bytes;
    use sha2::{Digest, Sha256};

    use super::{ByteSink, ChunkSource, StreamedDigest, transfer};

    struct Chunks {
        chunks: std::vec::IntoIter<Bytes>,
    }

    #[async_trait::async_trait]
    impl ChunkSource for Chunks {
        async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
            Ok(self.chunks.next())
        }
    }

    struct RecordingSink {
        written: Vec<u8>,
        fail_at: Option<usize>,
    }

    #[async_trait::async_trait]
    impl ByteSink for RecordingSink {
        async fn write_chunk(&mut self, chunk: Bytes) -> Result<()> {
            if self
                .fail_at
                .is_some_and(|limit| self.written.len() >= limit)
            {
                return Err(anyhow::anyhow!("staging object write failed"));
            }
            self.written.extend_from_slice(&chunk);
            Ok(())
        }
    }

    fn source(chunks: &[&[u8]]) -> Chunks {
        Chunks {
            chunks: chunks
                .iter()
                .map(|chunk| Bytes::copy_from_slice(chunk))
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }

    fn sink(fail_at: Option<usize>) -> RecordingSink {
        RecordingSink {
            written: Vec::new(),
            fail_at,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[tokio::test]
    async fn streams_every_chunk_and_reports_the_canonical_digest() {
        let body = b"issue 667 source materialization";
        let mut chunks = source(&[b"issue 667 ", b"source ", b"materialization"]);
        let mut sink = sink(None);

        let streamed: StreamedDigest = transfer(&mut chunks, &mut sink, 1024)
            .await
            .expect("stream the body");

        assert_eq!(sink.written, body);
        assert_eq!(streamed.size_bytes, body.len() as i64);
        assert_eq!(streamed.sha256, sha256_hex(body));
    }

    #[tokio::test]
    async fn enforces_the_size_limit_across_chunk_boundaries() {
        let mut chunks = source(&[b"1234", b"56"]);
        let mut sink = sink(None);

        let error = transfer(&mut chunks, &mut sink, 5)
            .await
            .expect_err("the body exceeds the limit");

        assert_eq!(error.to_string(), "remote_file_too_large");
        // The chunk that crossed the limit is never written to the sink.
        assert_eq!(sink.written, b"1234");
    }

    #[tokio::test]
    async fn a_sink_failure_stops_the_stream_with_the_store_error() {
        let mut chunks = source(&[b"first", b"second", b"third"]);
        let mut sink = sink(Some(5));

        let error = transfer(&mut chunks, &mut sink, 1024)
            .await
            .expect_err("the sink fails on the second chunk");

        assert!(error.to_string().contains("staging object write failed"));
        assert_eq!(sink.written, b"first");
    }

    #[tokio::test]
    async fn an_empty_body_streams_to_a_zero_length_object() {
        let mut chunks = source(&[]);
        let mut sink = sink(None);

        let streamed = transfer(&mut chunks, &mut sink, 1024)
            .await
            .expect("empty body");

        assert!(sink.written.is_empty());
        assert_eq!(streamed.size_bytes, 0);
        assert_eq!(streamed.sha256, sha256_hex(b""));
    }
}
