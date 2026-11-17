//! Magic-byte, media-type and bounded-size validation of the supported image
//! kinds.
//!
//! Validation runs before anything stores or forwards image bytes: the declared
//! media type and filename extension must name the same supported kind, the
//! bytes must carry that kind's magic, animation is refused, and the encoded
//! and decoded bounds are enforced from the file header alone.

use std::io::Cursor;

use crate::domain_errors::DomainError;

use super::LibraryImageKind;

/// The bounded image budgets one upload is validated against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageLimits {
    pub(crate) max_encoded_bytes: usize,
    pub(crate) max_dimension: u32,
    pub(crate) max_decoded_bytes: u64,
}

impl ImageLimits {
    /// The largest accepted width or height of one supported image.
    pub(crate) const DEFAULT_MAX_DIMENSION: u32 = 8192;

    /// The largest accepted `width * height * 4` decoded buffer.
    pub(crate) const DEFAULT_MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;

    pub(crate) const fn new(max_encoded_bytes: usize) -> Self {
        Self {
            max_encoded_bytes,
            max_dimension: Self::DEFAULT_MAX_DIMENSION,
            max_decoded_bytes: Self::DEFAULT_MAX_DECODED_BYTES,
        }
    }
}

/// An image's bounded dimensions, read from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageDimensions {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_MAGIC: [u8; 3] = [0xff, 0xd8, 0xff];
/// Image extensions Context69 refuses even though the bytes might decode.
const REFUSED_IMAGE_EXTENSIONS: [&str; 14] = [
    "apng", "avif", "bmp", "gif", "heic", "heif", "ico", "jfif", "jxl", "psd", "svg", "svgz",
    "tif", "tiff",
];

/// The supported image kind the bytes carry by magic, or `None`.
pub(crate) fn image_kind_from_bytes(bytes: &[u8]) -> Option<LibraryImageKind> {
    if bytes.starts_with(&PNG_MAGIC) {
        return Some(LibraryImageKind::Png);
    }
    if bytes.starts_with(&JPEG_MAGIC) {
        return Some(LibraryImageKind::Jpeg);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(LibraryImageKind::WebP);
    }
    None
}

/// The supported image kind a filename/media type pair declares.
///
/// `Ok(None)` means the upload does not claim to be an image at all, so the
/// existing document classification owns it unchanged. A claim for a refused
/// image format, or a media type and extension that disagree, is an explicit
/// invalid argument instead.
pub(crate) fn declared_image_kind(
    filename: &str,
    media_type: &str,
) -> anyhow::Result<Option<LibraryImageKind>> {
    let media_type = media_type.trim();
    let extension = filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.trim().to_ascii_lowercase());
    let media_kind = LibraryImageKind::from_media_type(media_type);
    let extension_kind = extension
        .as_deref()
        .and_then(LibraryImageKind::from_extension);
    let refused_media =
        media_type.to_ascii_lowercase().starts_with("image/") && media_kind.is_none();
    let refused_extension = extension.as_deref().is_some_and(|extension| {
        REFUSED_IMAGE_EXTENSIONS.contains(&extension)
            && LibraryImageKind::from_extension(extension).is_none()
    });
    if refused_media || refused_extension {
        return Err(DomainError::invalid_argument(format!(
            "unsupported image type for {filename}"
        ))
        .into());
    }
    match (media_kind, extension_kind) {
        (Some(media), Some(extension)) if media != extension => {
            Err(DomainError::invalid_argument(format!(
                "image media type {} and filename {filename} disagree",
                media.media_type()
            ))
            .into())
        }
        (Some(media), _) => Ok(Some(media)),
        (None, extension) => Ok(extension),
    }
}

/// The supported image kind an already stored file is identified by.
pub(crate) fn require_image_media(
    filename: &str,
    media_type: &str,
) -> anyhow::Result<LibraryImageKind> {
    declared_image_kind(filename, media_type)?.ok_or_else(|| {
        DomainError::invalid_argument(format!(
            "file {filename} is not a supported PNG, JPEG or WebP image"
        ))
        .into()
    })
}

/// Validate one image upload against the bounded image limits.
pub(crate) fn validate_image(
    filename: &str,
    media_type: &str,
    bytes: &[u8],
    limits: ImageLimits,
) -> anyhow::Result<LibraryImageKind> {
    if bytes.is_empty() {
        return Err(DomainError::invalid_argument(format!("image {filename} is empty")).into());
    }
    if bytes.len() > limits.max_encoded_bytes {
        return Err(DomainError::payload_too_large(format!(
            "image {filename} exceeds the image size limit of {} bytes",
            limits.max_encoded_bytes
        ))
        .into());
    }
    let declared = declared_image_kind(filename, media_type)?.ok_or_else(|| {
        DomainError::invalid_argument(format!(
            "unsupported image type for {filename}; expected PNG, JPEG or WebP"
        ))
    })?;
    let detected = image_kind_from_bytes(bytes).ok_or_else(|| {
        DomainError::invalid_argument(format!(
            "image {filename} does not contain PNG, JPEG or WebP bytes"
        ))
    })?;
    if declared != detected {
        return Err(DomainError::invalid_argument(format!(
            "image {filename} declares {} but contains {} bytes",
            declared.media_type(),
            detected.media_type()
        ))
        .into());
    }
    let animated = match detected {
        LibraryImageKind::Png => is_animated_png(bytes),
        LibraryImageKind::WebP => is_animated_webp(bytes),
        LibraryImageKind::Jpeg => false,
    };
    if animated {
        return Err(DomainError::invalid_argument(format!(
            "animated image {filename} is not supported"
        ))
        .into());
    }
    let dimensions = image_dimensions(filename, detected, bytes, limits)?;
    if dimensions.width == 0 || dimensions.height == 0 {
        return Err(
            DomainError::invalid_argument(format!("image {filename} has no pixels")).into(),
        );
    }
    if dimensions.width > limits.max_dimension || dimensions.height > limits.max_dimension {
        return Err(DomainError::payload_too_large(format!(
            "image {filename} is {}x{}; the limit is {} pixels per side",
            dimensions.width, dimensions.height, limits.max_dimension
        ))
        .into());
    }
    let decoded_bytes = u64::from(dimensions.width) * u64::from(dimensions.height) * 4;
    if decoded_bytes > limits.max_decoded_bytes {
        return Err(DomainError::payload_too_large(format!(
            "image {filename} needs {decoded_bytes} decoded bytes; the limit is {}",
            limits.max_decoded_bytes
        ))
        .into());
    }
    Ok(detected)
}

fn image_dimensions(
    filename: &str,
    kind: LibraryImageKind,
    bytes: &[u8],
    limits: ImageLimits,
) -> anyhow::Result<ImageDimensions> {
    let mut image_limits = image::Limits::no_limits();
    image_limits.max_image_width = Some(limits.max_dimension);
    image_limits.max_image_height = Some(limits.max_dimension);
    image_limits.max_alloc = Some(limits.max_decoded_bytes);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image_format(kind));
    reader.limits(image_limits);
    let (width, height) = reader.into_dimensions().map_err(|error| match error {
        image::ImageError::Limits(_) => DomainError::payload_too_large(format!(
            "image {filename} exceeds the {} pixel limit per side",
            limits.max_dimension
        )),
        other => DomainError::invalid_argument(format!("invalid {:?} image header: {other}", kind)),
    })?;
    Ok(ImageDimensions { width, height })
}

const fn image_format(kind: LibraryImageKind) -> image::ImageFormat {
    match kind {
        LibraryImageKind::Png => image::ImageFormat::Png,
        LibraryImageKind::Jpeg => image::ImageFormat::Jpeg,
        LibraryImageKind::WebP => image::ImageFormat::WebP,
    }
}

/// An animated PNG is declared by an `acTL` chunk before the first `IDAT`.
fn is_animated_png(bytes: &[u8]) -> bool {
    let mut offset = PNG_MAGIC.len();
    while let Some(next) = offset.checked_add(8).filter(|next| *next <= bytes.len()) {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]));
        let kind = &bytes[offset + 4..next];
        if kind == b"acTL" {
            return true;
        }
        if kind == b"IDAT" {
            return false;
        }
        let Some(next_offset) = next
            .checked_add(length as usize)
            .and_then(|offset| offset.checked_add(4))
        else {
            return false;
        };
        offset = next_offset;
    }
    false
}

/// An animated WebP is declared either by the `VP8X` animation flag or by an
/// `ANIM` chunk.
fn is_animated_webp(bytes: &[u8]) -> bool {
    if bytes.len() >= 21 && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0 {
        return true;
    }
    bytes.windows(4).any(|window| window == b"ANIM")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::domain_errors::{DomainError, find_domain_error};

    use super::{
        ImageLimits, LibraryImageKind, declared_image_kind, image_dimensions,
        image_kind_from_bytes, is_animated_png, is_animated_webp, require_image_media,
        validate_image,
    };

    fn encoded(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut output, format)
            .expect("the test image encodes");
        output.into_inner()
    }

    #[test]
    fn supported_magic_bytes_classify_each_kind() {
        assert_eq!(
            image_kind_from_bytes(&encoded(1, 1, image::ImageFormat::Png)),
            Some(LibraryImageKind::Png)
        );
        assert_eq!(
            image_kind_from_bytes(&encoded(1, 1, image::ImageFormat::Jpeg)),
            Some(LibraryImageKind::Jpeg)
        );
        assert_eq!(
            image_kind_from_bytes(&encoded(1, 1, image::ImageFormat::WebP)),
            Some(LibraryImageKind::WebP)
        );
        assert_eq!(image_kind_from_bytes(b"plain text"), None);
    }

    #[test]
    fn declared_kind_requires_media_type_and_extension_agreement() {
        assert_eq!(
            declared_image_kind("cover.PNG", "image/png").unwrap(),
            Some(LibraryImageKind::Png)
        );
        assert_eq!(
            declared_image_kind("cover.jpg", "image/jpeg").unwrap(),
            Some(LibraryImageKind::Jpeg)
        );
        assert_eq!(
            declared_image_kind("report.pdf", "application/pdf").unwrap(),
            None
        );
        assert_eq!(
            declared_image_kind("photo.jpg", "image/png")
                .unwrap_err()
                .to_string(),
            "image media type image/png and filename photo.jpg disagree"
        );
        for (filename, media_type) in [
            ("icon.svg", "image/svg+xml"),
            ("icon.svg", "application/octet-stream"),
            ("photo.gif", "image/gif"),
            ("photo.png", "image/gif"),
        ] {
            let error = declared_image_kind(filename, media_type).unwrap_err();
            assert!(
                matches!(
                    find_domain_error(&error),
                    Some(DomainError::InvalidArgument(_))
                ),
                "{filename}/{media_type}: {error}"
            );
        }
    }

    #[test]
    fn stored_file_media_must_be_a_supported_image() {
        assert_eq!(
            require_image_media("cover.webp", "image/webp").unwrap(),
            LibraryImageKind::WebP
        );
        assert!(require_image_media("report.pdf", "application/pdf").is_err());
    }

    #[test]
    fn valid_images_report_their_dimensions() {
        let limits = ImageLimits::new(1024 * 1024);
        for (format, media_type, kind) in [
            (image::ImageFormat::Png, "image/png", LibraryImageKind::Png),
            (
                image::ImageFormat::Jpeg,
                "image/jpeg",
                LibraryImageKind::Jpeg,
            ),
            (
                image::ImageFormat::WebP,
                "image/webp",
                LibraryImageKind::WebP,
            ),
        ] {
            let bytes = encoded(3, 2, format);
            let validated =
                validate_image("figure.img", media_type, &bytes, limits).expect("valid image");
            assert_eq!(validated, kind);
            let dimensions = image_dimensions("figure.img", kind, &bytes, limits)
                .expect("the header is readable");
            assert_eq!((dimensions.width, dimensions.height), (3, 2));
        }
    }

    #[test]
    fn mismatched_and_corrupt_images_fail_explicitly() {
        let limits = ImageLimits::new(1024 * 1024);
        let png = encoded(1, 1, image::ImageFormat::Png);
        let error = validate_image("cover.img", "image/jpeg", &png, limits).unwrap_err();
        assert_eq!(
            error.to_string(),
            "image cover.img declares image/jpeg but contains image/png bytes"
        );
        let error = validate_image("cover.png", "image/jpeg", &png, limits).unwrap_err();
        assert_eq!(
            error.to_string(),
            "image media type image/jpeg and filename cover.png disagree"
        );
        let error = validate_image("cover.png", "image/png", b"not an image", limits).unwrap_err();
        assert_eq!(
            error.to_string(),
            "image cover.png does not contain PNG, JPEG or WebP bytes"
        );
        let truncated = &png[..png.len() / 2];
        let error = validate_image("cover.png", "image/png", truncated, limits).unwrap_err();
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::InvalidArgument(_))
            ),
            "{error}"
        );
    }

    #[test]
    fn encoded_and_decoded_bounds_are_enforced() {
        let png = encoded(4, 4, image::ImageFormat::Png);
        let error =
            validate_image("cover.png", "image/png", &png, ImageLimits::new(8)).unwrap_err();
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::PayloadTooLarge(_))
            ),
            "{error}"
        );

        let mut limits = ImageLimits::new(1024 * 1024);
        limits.max_dimension = 2;
        let error = validate_image("cover.png", "image/png", &png, limits).unwrap_err();
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::PayloadTooLarge(_))
            ),
            "{error}"
        );

        let mut limits = ImageLimits::new(1024 * 1024);
        limits.max_decoded_bytes = 4 * 4 * 4 - 1;
        let error = validate_image("cover.png", "image/png", &png, limits).unwrap_err();
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::PayloadTooLarge(_))
            ),
            "{error}"
        );
    }

    #[test]
    fn animated_images_are_refused() {
        let mut animated_png = Vec::from(super::PNG_MAGIC);
        animated_png.extend_from_slice(&0u32.to_be_bytes());
        animated_png.extend_from_slice(b"acTL");
        animated_png.extend_from_slice(&[0; 4]);
        assert!(is_animated_png(&animated_png));
        let error = validate_image(
            "cover.png",
            "image/png",
            &animated_png,
            ImageLimits::new(1024 * 1024),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "animated image cover.png is not supported"
        );

        let mut animated_webp = Vec::new();
        animated_webp.extend_from_slice(b"RIFF");
        animated_webp.extend_from_slice(&0u32.to_le_bytes());
        animated_webp.extend_from_slice(b"WEBP");
        animated_webp.extend_from_slice(b"VP8X");
        animated_webp.extend_from_slice(&10u32.to_le_bytes());
        animated_webp.extend_from_slice(&[0x02, 0, 0, 0]);
        animated_webp.extend_from_slice(&[0; 10]);
        assert_eq!(
            image_kind_from_bytes(&animated_webp),
            Some(LibraryImageKind::WebP)
        );
        assert!(is_animated_webp(&animated_webp));
        let error = validate_image(
            "cover.webp",
            "image/webp",
            &animated_webp,
            ImageLimits::new(1024 * 1024),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "animated image cover.webp is not supported"
        );
    }
}
