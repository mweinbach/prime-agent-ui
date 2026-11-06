//! Immutable snapshots shared by the preview, composer draft and daemon request.
use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use base64::Engine as _;
use serde_json::{Value, json};
use uuid::Uuid;

use super::image_preview::{self, ImagePreview};

pub(crate) const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 15 * 1024 * 1024;
pub(crate) const MAX_ATTACHMENTS: usize = 16;
const TEXT_PREVIEW_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub enum AttachmentPreview {
    Image(ImagePreview),
    Text { excerpt: String, truncated: bool },
}

#[derive(Clone)]
enum Payload {
    Image {
        mime: &'static str,
        bytes: Arc<[u8]>,
    },
    Text(Arc<str>),
}

#[derive(Clone)]
pub struct StagedAttachment {
    pub id: Uuid,
    pub filename: String,
    pub byte_len: usize,
    pub preview: AttachmentPreview,
    payload: Payload,
}

// Avoid logging prompt contents or image data when a bridge request is inspected.
impl std::fmt::Debug for StagedAttachment {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("StagedAttachment")
            .field("id", &self.id)
            .field("byte_len", &self.byte_len)
            .finish_non_exhaustive()
    }
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn escaped_name(filename: &str) -> String {
    filename
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl StagedAttachment {
    pub(crate) fn read(path: &Path, remaining: usize) -> Result<Self> {
        let filename = path
            .file_name()
            .context("Choose a file")?
            .to_string_lossy()
            .into_owned();
        ensure!(path.metadata()?.is_file(), "Choose a regular file.");
        let file = File::open(path).with_context(|| format!("Could not open {filename}"))?;
        let meta = file.metadata()?;
        ensure!(meta.is_file(), "Choose a regular file.");
        ensure!(
            meta.len() <= remaining as u64,
            "Attachments exceed the 15 MiB limit."
        );
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(remaining as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= remaining,
            "Attachments exceed the 15 MiB limit."
        );
        let byte_len = bytes.len();
        let (preview, payload) = if let Some(mime) = image_mime(&bytes) {
            let preview = image_preview::decode(&bytes, mime)
                .with_context(|| format!("Cannot preview {filename}"))?;
            (
                AttachmentPreview::Image(preview),
                Payload::Image {
                    mime,
                    bytes: bytes.into(),
                },
            )
        } else {
            let ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let binary_extension = matches!(
                ext.as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "pdf"
                    | "doc"
                    | "docx"
                    | "xls"
                    | "xlsx"
                    | "ppt"
                    | "pptx"
                    | "zip"
                    | "odt"
                    | "ods"
                    | "odp"
            );
            ensure!(
                !binary_extension && !bytes.trim_ascii_start().starts_with(b"%PDF-"),
                "{filename} is not a supported image or UTF-8 text file."
            );
            ensure!(
                byte_len <= MAX_PROMPT_BYTES,
                "{filename} exceeds the 1 MiB text prompt limit."
            );
            let text = String::from_utf8(bytes)
                .with_context(|| format!("{filename} is not a UTF-8 text file."))?;
            ensure!(
                !text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{000c}')),
                "{filename} contains binary data. Choose a UTF-8 text file."
            );
            let end = text.floor_char_boundary(TEXT_PREVIEW_BYTES.min(text.len()));
            let preview = AttachmentPreview::Text {
                excerpt: text[..end].to_owned(),
                truncated: end != text.len(),
            };
            (preview, Payload::Text(text.into()))
        };
        Ok(Self {
            id: Uuid::new_v4(),
            filename,
            byte_len,
            preview,
            payload,
        })
    }

    #[must_use]
    pub fn image_mime(&self) -> Option<&'static str> {
        match &self.payload {
            Payload::Image { mime, .. } => Some(mime),
            Payload::Text(_) => None,
        }
    }

    pub(crate) fn image_source(&self) -> Option<(&'static str, Arc<[u8]>)> {
        if let Payload::Image { mime, bytes } = &self.payload {
            Some((*mime, Arc::clone(bytes)))
        } else {
            None
        }
    }

    pub(crate) fn wire_image(&self) -> Option<Value> {
        let Payload::Image { mime, bytes } = &self.payload else {
            return None;
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(json!({"type":"image", "mimeType":mime, "data": encoded}))
    }

    fn file_block_size(&self) -> usize {
        match &self.payload {
            Payload::Text(text) => {
                "<file name=\"\">\n\n</file>\n".len()
                    + escaped_name(&self.filename).len()
                    + text.len()
            }
            Payload::Image { .. } => 0,
        }
    }
}

pub(crate) fn prompt_size(message: &str, attachments: &[StagedAttachment]) -> usize {
    let blocks: usize = attachments
        .iter()
        .map(StagedAttachment::file_block_size)
        .sum();
    blocks + message.len() + usize::from(blocks > 0 && !message.is_empty()) * 2
}

/// The CLI's file-block convention, followed by the user's instructions. Images
/// keep their separate daemon content objects and do not change the typed text.
pub(crate) fn wire_text<'a>(message: &'a str, attachments: &[StagedAttachment]) -> Cow<'a, str> {
    let mut blocks = String::new();
    for attachment in attachments {
        if let Payload::Text(text) = &attachment.payload {
            let name = escaped_name(&attachment.filename);
            let _ = writeln!(&mut blocks, "<file name=\"{name}\">\n{text}\n</file>");
        }
    }
    if blocks.is_empty() {
        return Cow::Borrowed(message);
    }
    if !message.is_empty() {
        blocks.push_str("\n\n");
        blocks.push_str(message);
    }
    Cow::Owned(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_and_message_share_one_snapshot_of_the_file() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join("notes & plans.txt");
        let content = format!("{}🌸\nend\r\n", "a".repeat(TEXT_PREVIEW_BYTES - 1));
        std::fs::write(&path, &content).unwrap();
        let file = StagedAttachment::read(&path, MAX_ATTACHMENT_BYTES).unwrap();
        std::fs::write(&path, "later edits").unwrap();
        let AttachmentPreview::Text { excerpt, truncated } = &file.preview else {
            panic!("text preview");
        };
        assert!(*truncated);
        assert_eq!(excerpt, &content[..TEXT_PREVIEW_BYTES - 1]);
        assert_eq!(file.byte_len, content.len());
        let files = [file];
        let expected = format!("<file name=\"notes &amp; plans.txt\">\n{content}\n</file>\n");
        assert_eq!(wire_text("", &files), expected);
        assert_eq!(wire_text("Review", &files), format!("{expected}\n\nReview"));
        assert_eq!(
            prompt_size("Review", &files),
            wire_text("Review", &files).len()
        );
        assert!(files[0].wire_image().is_none());
    }

    #[test]
    fn unsupported_or_oversized_files_cannot_be_sent_as_text() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join("plain.txt");
        for bad in [&[0xff, 0xfe][..], b"ABC\0DEF", b"%PDF-1.7\n1 0 obj"] {
            std::fs::write(&path, bad).unwrap();
            assert!(StagedAttachment::read(&path, MAX_ATTACHMENT_BYTES).is_err());
        }
        std::fs::write(&path, b"file").unwrap();
        assert!(StagedAttachment::read(&path, 3).is_err());
        assert!(StagedAttachment::read(&path, 4).is_ok());
        let fake_image = profile.path().join("wrong.png");
        std::fs::write(&fake_image, b"not an image").unwrap();
        assert!(StagedAttachment::read(&fake_image, MAX_ATTACHMENT_BYTES).is_err());
    }

    #[test]
    fn magic_identifies_image_bytes_and_preserves_original_payload() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join("misnamed.txt");
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[24, 48, 96], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(&path, &jpeg).unwrap();
        let file = StagedAttachment::read(&path, MAX_ATTACHMENT_BYTES).unwrap();
        assert!(matches!(file.preview, AttachmentPreview::Image(_)));
        assert_eq!(
            file.wire_image(),
            Some(json!({
                "type":"image",
                "mimeType":"image/jpeg",
                "data":base64::engine::general_purpose::STANDARD.encode(jpeg),
            }))
        );
        assert_eq!(wire_text("Describe", &[file]), "Describe");
    }
}
