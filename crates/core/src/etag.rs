use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ETag(String);

impl ETag {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn etag_for_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("\"{}\"", hex_lower(&digest))
}

pub fn typed_etag_for_bytes(bytes: &[u8]) -> ETag {
    ETag(etag_for_bytes(bytes))
}

pub fn etag(input: &str) -> String {
    etag_for_bytes(input.as_bytes())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
