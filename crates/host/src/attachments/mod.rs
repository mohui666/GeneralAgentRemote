use std::{
    fs,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
};

use agent_remote_protocol::{AttachmentId, AttachmentMetadata, ConversationId};
use anyhow::{Context, Result, bail};
use image::{GenericImageView, ImageFormat, ImageReader};

use crate::storage::{StoredAttachment, now_ms};

pub const DEFAULT_MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

pub struct AttachmentStore {
    root: PathBuf,
    max_image_bytes: u64,
}

impl AttachmentStore {
    pub fn new(root: impl AsRef<Path>, max_image_bytes: u64) -> Result<Self> {
        fs::create_dir_all(root.as_ref())
            .with_context(|| format!("create attachment directory {}", root.as_ref().display()))?;
        Ok(Self {
            root: root.as_ref().canonicalize()?,
            max_image_bytes,
        })
    }

    pub fn import_file(
        &self,
        conversation_id: ConversationId,
        source: &Path,
        allowed_roots: &[PathBuf],
    ) -> Result<StoredAttachment> {
        let canonical = source
            .canonicalize()
            .with_context(|| format!("image path does not exist: {}", source.display()))?;
        let allowed = allowed_roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|canonical_root| canonical.starts_with(canonical_root))
        });
        if !allowed {
            bail!("image path is outside the project and controlled temporary roots");
        }
        let length = fs::metadata(&canonical)?.len();
        if length > self.max_image_bytes {
            bail!(
                "image exceeds the configured {} byte limit",
                self.max_image_bytes
            );
        }
        let bytes = fs::read(&canonical)?;
        self.import_bytes(conversation_id, &bytes, None)
    }

    pub fn import_bytes(
        &self,
        conversation_id: ConversationId,
        bytes: &[u8],
        declared_mime: Option<&str>,
    ) -> Result<StoredAttachment> {
        if bytes.len() as u64 > self.max_image_bytes {
            bail!(
                "image exceeds the configured {} byte limit",
                self.max_image_bytes
            );
        }
        let format = image::guess_format(bytes).context("unrecognized image signature")?;
        let (mime_type, extension) = allowed_format(format)?;
        if let Some(declared) = declared_mime
            && declared != mime_type
        {
            bail!("declared image type {declared} does not match detected type {mime_type}");
        }
        let reader = ImageReader::with_format(BufReader::new(Cursor::new(bytes)), format);
        let image = reader.decode().context("image failed to decode")?;
        let (width, height) = image.dimensions();
        let id = AttachmentId::new();
        let managed_path = self.root.join(format!("{id}.{extension}"));
        fs::write(&managed_path, bytes)
            .with_context(|| format!("write managed attachment {}", managed_path.display()))?;
        Ok(StoredAttachment {
            metadata: AttachmentMetadata {
                id,
                conversation_id,
                mime_type: mime_type.to_owned(),
                byte_len: bytes.len() as u64,
                width: Some(width),
                height: Some(height),
                created_at_ms: now_ms(),
            },
            managed_path,
        })
    }

    pub fn read(&self, attachment: &StoredAttachment) -> Result<Vec<u8>> {
        let canonical = attachment.managed_path.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            bail!("attachment path is outside the managed directory");
        }
        let length = fs::metadata(&canonical)?.len();
        if length > self.max_image_bytes {
            bail!("managed image exceeds the configured limit");
        }
        Ok(fs::read(canonical)?)
    }
}

fn allowed_format(format: ImageFormat) -> Result<(&'static str, &'static str)> {
    match format {
        ImageFormat::Png => Ok(("image/png", "png")),
        ImageFormat::Jpeg => Ok(("image/jpeg", "jpg")),
        ImageFormat::WebP => Ok(("image/webp", "webp")),
        ImageFormat::Gif => Ok(("image/gif", "gif")),
        _ => bail!("image format is not allowed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn png_bytes() -> Vec<u8> {
        let image = ImageBuffer::from_pixel(2, 3, Rgba([12_u8, 34, 56, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }

    #[test]
    fn valid_png_is_copied_under_random_attachment_id() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = AttachmentStore::new(temp.path().join("attachments"), 1024 * 1024)
            .expect("attachment store");
        let attachment = store
            .import_bytes(ConversationId::new(), &png_bytes(), Some("image/png"))
            .expect("import png");
        assert_eq!(attachment.metadata.width, Some(2));
        assert_eq!(attachment.metadata.height, Some(3));
        assert!(
            attachment
                .managed_path
                .ends_with(format!("{}.png", attachment.metadata.id))
        );
        assert_eq!(store.read(&attachment).expect("read image"), png_bytes());
    }

    #[test]
    fn oversized_and_dangerous_content_is_rejected() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store =
            AttachmentStore::new(temp.path().join("attachments"), 8).expect("attachment store");
        assert!(
            store
                .import_bytes(ConversationId::new(), &png_bytes(), None)
                .is_err()
        );
        assert!(
            store
                .import_bytes(ConversationId::new(), b"<svg></svg>", Some("image/svg+xml"))
                .is_err()
        );
    }
}
