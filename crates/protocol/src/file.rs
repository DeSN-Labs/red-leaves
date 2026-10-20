//! Bounded manifests and hash pages. Filesystem persistence and access policy live in node.

use serde::{Deserialize, Serialize};

use crate::{ContentId, Error, MAX_CHUNK, MAX_OBJECT, Result, canonical};

pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const HASHES_PER_PAGE: usize = 128;
pub const MAX_HASH_PAGES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileManifest {
    /// Hash of the exact transferred bytes (ciphertext for encrypted files).
    pub object_id: ContentId,
    pub total_length: u64,
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub hash_page_ids: Vec<ContentId>,
    pub media_type: String,
}

impl FileManifest {
    pub fn validate(&self) -> Result<()> {
        ContentId::from_slice(self.object_id.as_bytes())?;
        if self.total_length == 0
            || self.total_length > MAX_FILE_BYTES
            || self.chunk_size == 0
            || self.chunk_size > MAX_CHUNK as u32
            || u64::from(self.chunk_count) != self.total_length.div_ceil(u64::from(self.chunk_size))
            || self.hash_page_ids.len() != (self.chunk_count as usize).div_ceil(HASHES_PER_PAGE)
            || self.hash_page_ids.len() > MAX_HASH_PAGES
        {
            return Err(Error::ResourceExhausted);
        }
        if self.media_type.is_empty()
            || self.media_type.len() > 128
            || !self.media_type.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(Error::InvalidMessage);
        }
        for id in &self.hash_page_ids {
            ContentId::from_slice(id.as_bytes())?;
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical::encode(self, MAX_OBJECT)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Self = canonical::decode(bytes, MAX_OBJECT)?;
        value.validate()?;
        Ok(value)
    }

    pub fn verify_page(&self, index: u32, bytes: &[u8]) -> Result<HashPage> {
        self.validate()?;
        if bytes.len() > MAX_OBJECT {
            return Err(Error::ResourceExhausted);
        }
        self.hash_page_ids
            .get(index as usize)
            .ok_or(Error::InvalidMessage)?
            .verify(bytes)?;
        let page = HashPage::decode(bytes)?;
        let first_chunk = index as usize * HASHES_PER_PAGE;
        let expected_count = (self.chunk_count as usize - first_chunk).min(HASHES_PER_PAGE);
        if page.object_id != self.object_id
            || page.page_index != index
            || page.chunk_ids.len() != expected_count
        {
            return Err(Error::IntegrityFailed);
        }
        Ok(page)
    }

    pub fn verify_chunk(&self, index: u32, bytes: &[u8], page_bytes: &[u8]) -> Result<()> {
        self.validate()?;
        if index >= self.chunk_count {
            return Err(Error::InvalidMessage);
        }
        let page = self.verify_page(index / HASHES_PER_PAGE as u32, page_bytes)?;
        let expected_length = (self.total_length - u64::from(index) * u64::from(self.chunk_size))
            .min(u64::from(self.chunk_size));
        if bytes.len() as u64 != expected_length {
            return Err(Error::IntegrityFailed);
        }
        page.chunk_ids[index as usize % HASHES_PER_PAGE].verify(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashPage {
    pub object_id: ContentId,
    pub page_index: u32,
    pub chunk_ids: Vec<ContentId>,
}

impl HashPage {
    fn validate(&self) -> Result<()> {
        ContentId::from_slice(self.object_id.as_bytes())?;
        if self.page_index as usize >= MAX_HASH_PAGES
            || self.chunk_ids.is_empty()
            || self.chunk_ids.len() > HASHES_PER_PAGE
        {
            return Err(Error::ResourceExhausted);
        }
        for id in &self.chunk_ids {
            ContentId::from_slice(id.as_bytes())?;
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        canonical::encode(self, MAX_OBJECT)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Self = canonical::decode(bytes, MAX_OBJECT)?;
        value.validate()?;
        Ok(value)
    }
}
