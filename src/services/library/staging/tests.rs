//! Tests for the Docling capacity helpers owned by the enclosing `library`
//! module.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::super::resize_docling_semaphore;

#[test]
fn docling_resize_shrink_fail_then_grow_matches_latest_target() {
    // Regression test for review note 4908: a shrink that cannot
    // reclaim checked-out permits must report the actual total so the
    // next grow deltas from reality instead of over-adding.
    let semaphore = Arc::new(Semaphore::new(4));
    let held: Vec<OwnedSemaphorePermit> = (0..4)
        .map(|_| {
            semaphore
                .clone()
                .try_acquire_owned()
                .expect("initial permit")
        })
        .collect();
    assert_eq!(semaphore.available_permits(), 0);

    let actual = resize_docling_semaphore(&semaphore, 4, 2);
    assert_eq!(
        actual, 4,
        "failed shrink must report the actual total, not the desired target"
    );
    assert_eq!(semaphore.available_permits(), 0);

    let actual = resize_docling_semaphore(&semaphore, actual, 5);
    assert_eq!(actual, 5, "grow must delta from the actual total");
    assert_eq!(
        semaphore.available_permits() + held.len(),
        5,
        "semaphore total must equal the latest target, not over-admit"
    );
    drop(held);
    assert_eq!(semaphore.available_permits(), 5);
}

#[test]
fn docling_resize_shrink_reclaims_idle_permits_exactly() {
    let semaphore = Arc::new(Semaphore::new(4));
    let actual = resize_docling_semaphore(&semaphore, 4, 2);
    assert_eq!(actual, 2);
    assert_eq!(semaphore.available_permits(), 2);
}

mod task_input_dispatch {
    use std::io::Cursor;

    use bytes::Bytes;
    use context69_contracts::tasks::TaskKind;

    use super::super::TaskInputMedia;
    use crate::services::library::UploadedLibraryFile;

    const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

    fn upload(filename: &str, media_type: &str, bytes: Vec<u8>) -> UploadedLibraryFile {
        UploadedLibraryFile {
            folder_id: None,
            filename: filename.to_string(),
            media_type: media_type.to_string(),
            bytes: Bytes::from(bytes),
            declared_sha256: None,
            options: context69_contracts::IngestOptions::retain(),
            staged_storage_object_id: None,
        }
    }

    fn png_bytes() -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut output, image::ImageFormat::Png)
            .expect("test png encodes");
        output.into_inner()
    }

    #[test]
    fn the_image_batch_dispatch_selects_image_validation() {
        assert_eq!(
            TaskInputMedia::for_task_kind(TaskKind::ImageBatch),
            TaskInputMedia::Image
        );
        for kind in [
            TaskKind::FileBatch,
            TaskKind::UrlBatch,
            TaskKind::TextBatch,
            TaskKind::SourceSync,
        ] {
            assert_eq!(
                TaskInputMedia::for_task_kind(kind),
                TaskInputMedia::Document,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_supported_png_passes_image_staging_and_is_refused_by_document_staging() {
        let image = upload("cover.png", "image/png", png_bytes());
        assert!(
            TaskInputMedia::Image
                .validate(&image, MAX_UPLOAD_BYTES)
                .is_ok()
        );
        assert!(
            TaskInputMedia::Document
                .validate(&image, MAX_UPLOAD_BYTES)
                .is_err()
        );
    }

    #[test]
    fn a_plain_text_upload_keeps_document_staging_and_fails_image_staging() {
        let document = upload("notes.txt", "text/plain", b"plain text".to_vec());
        assert!(
            TaskInputMedia::Document
                .validate(&document, MAX_UPLOAD_BYTES)
                .is_ok()
        );
        assert!(
            TaskInputMedia::Image
                .validate(&document, MAX_UPLOAD_BYTES)
                .is_err()
        );
    }

    #[test]
    fn an_image_claim_with_wrong_bytes_still_fails_image_staging() {
        let broken = upload("cover.png", "image/png", b"not a png".to_vec());
        assert!(
            TaskInputMedia::Image
                .validate(&broken, MAX_UPLOAD_BYTES)
                .is_err()
        );
    }
}
