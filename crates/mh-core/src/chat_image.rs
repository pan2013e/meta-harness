use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

pub const MAX_CHAT_IMAGES: usize = 4;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_ENCODED_IMAGE_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
pub const MAX_CHAT_REQUEST_BYTES: usize =
    MAX_CHAT_IMAGES * MAX_ENCODED_IMAGE_BYTES + 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct ChatImageInput {
    mime_type: String,
    data: String,
}

pub struct ChatImage {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

impl ChatImage {
    pub fn decode(input: ChatImageInput) -> Result<Self, &'static str> {
        if input.data.len() > MAX_ENCODED_IMAGE_BYTES {
            return Err("Each image must be at most 5 MiB.");
        }
        let bytes = STANDARD
            .decode(input.data)
            .map_err(|_| "Invalid image encoding.")?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err("Each image must be at most 5 MiB.");
        }
        let valid = match input.mime_type.as_str() {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
            "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
            "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
            _ => return Err("Paste a PNG, JPEG, GIF, or WebP image."),
        };
        if !valid {
            return Err("The image data does not match its declared type.");
        }
        Ok(Self {
            mime_type: input.mime_type,
            bytes,
        })
    }

    pub fn encode(&self) -> String {
        STANDARD.encode(&self.bytes)
    }
}
