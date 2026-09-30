//! Regression tests for streamed URL source materialization: the S3
//! dependency gate must only ever see object-store failures.

use anyhow::Result;
use bytes::Bytes;

use super::{StorageFailureSink, attributes_stream_failure};
use crate::services::library::dependency_errors::{is_configuration_error, is_s3_transient_error};
use crate::services::library::dependency_storage::is_storage_gate_failure;
use crate::services::library::streaming::{ByteSink, ChunkSource, transfer};

/// Yields one chunk, or fails with a remote-shaped error.
struct Source {
    chunk: Option<&'static [u8]>,
    error: Option<&'static str>,
}

#[async_trait::async_trait]
impl ChunkSource for Source {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        match self.error.take() {
            Some(message) => Err(anyhow::anyhow!("{message}")),
            None => Ok(self.chunk.take().map(Bytes::from_static)),
        }
    }
}

/// Accepts every chunk unless a write error is configured.
struct Sink {
    error: Option<&'static str>,
    written: Vec<u8>,
}

#[async_trait::async_trait]
impl ByteSink for Sink {
    async fn write_chunk(&mut self, chunk: Bytes) -> Result<()> {
        if let Some(message) = self.error.take() {
            return Err(anyhow::anyhow!("{message}"));
        }
        self.written.extend_from_slice(&chunk);
        Ok(())
    }
}

/// Regression for review note 10310 (P1): the streamed transfer must not
/// attribute a remote download failure to the object store.
///
/// The remote message here is the worst case — it is exactly what the S3
/// gate classifiers read as a configuration failure — so the only thing
/// keeping the gate untouched is the provenance flag.
#[tokio::test]
async fn a_remote_download_failure_is_not_attributed_to_the_object_store() {
    let mut source = Source {
        chunk: None,
        error: Some("remote_http_status_401"),
    };
    let mut inner = Sink {
        error: None,
        written: Vec::new(),
    };
    let mut sink = StorageFailureSink::new(&mut inner);

    let error = transfer(&mut source, &mut sink, 1024)
        .await
        .expect_err("the remote read fails");

    assert_eq!(error.to_string(), "remote_http_status_401");
    assert!(
        !sink.storage_failed,
        "a remote failure must not be reported as a storage event"
    );
    assert!(
        is_configuration_error(&error) || is_s3_transient_error(&error),
        "the message alone classifies as an S3 failure, which is why provenance decides"
    );
    assert!(
        !attributes_stream_failure("s3", sink.storage_failed, &error),
        "the stream decision must keep the remote failure away from the S3 gate"
    );
    assert!(
        !attributes_stream_failure("s3", true, &error),
        "even a wrongly-set flag cannot trip the gate for a non-object-store error"
    );
    assert!(inner.written.is_empty());
}

/// The same boundary for the size limit: an oversized remote body fails in
/// the transfer accounting, not in the object store.
#[tokio::test]
async fn a_size_limit_failure_is_not_attributed_to_the_object_store() {
    let mut source = Source {
        chunk: Some(b"0123456789"),
        error: None,
    };
    let mut inner = Sink {
        error: None,
        written: Vec::new(),
    };
    let mut sink = StorageFailureSink::new(&mut inner);

    let error = transfer(&mut source, &mut sink, 4)
        .await
        .expect_err("the body exceeds the limit");

    assert_eq!(error.to_string(), "remote_file_too_large");
    assert!(!sink.storage_failed);
    assert!(!attributes_stream_failure(
        "s3",
        sink.storage_failed,
        &error
    ));
    assert!(inner.written.is_empty());
}

/// An object-store write failure is still reported, and only the sink side
/// sets the flag.
#[tokio::test]
async fn an_object_store_write_failure_is_attributed_to_the_sink() {
    let mut source = Source {
        chunk: Some(b"body"),
        error: None,
    };
    let mut inner = Sink {
        error: Some("s3 operation writer_write failed: kind=Unexpected: connection reset"),
        written: Vec::new(),
    };
    let mut sink = StorageFailureSink::new(&mut inner);

    let error = transfer(&mut source, &mut sink, 1024)
        .await
        .expect_err("the sink write fails");

    assert!(error.to_string().contains("s3 operation writer_write"));
    assert!(
        sink.storage_failed,
        "a sink write failure is a storage event"
    );
    assert!(is_storage_gate_failure("s3", &error));
    assert!(
        attributes_stream_failure("s3", sink.storage_failed, &error),
        "an object-store failure must still reach the S3 gate"
    );
}
