//! Restricted RFC 8949 core deterministic CBOR for signed payloads.
//! Text map keys, unsigned integers, bytes, arrays, booleans and null only.

use ciborium::value::Value;
use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, Result};

const MAX_DEPTH: usize = 16;
const MAX_ITEMS: usize = 4096;

pub fn encode<T: Serialize>(value: &T, max_bytes: usize) -> Result<Vec<u8>> {
    let mut value = Value::serialized(value).map_err(|_| Error::InvalidMessage)?;
    normalize(&mut value, 0)?;
    let mut bytes = Vec::new();
    ciborium::into_writer(&value, &mut bytes).map_err(|_| Error::InvalidMessage)?;
    preflight(&bytes, max_bytes)?;
    Ok(bytes)
}

pub fn decode<T: DeserializeOwned + Serialize>(bytes: &[u8], max_bytes: usize) -> Result<T> {
    // Bound nesting, counts and lengths before allocating the CBOR value tree.
    preflight(bytes, max_bytes)?;
    let mut value: Value = ciborium::de::from_reader_with_recursion_limit(bytes, MAX_DEPTH + 2)
        .map_err(|_| Error::InvalidMessage)?;
    normalize(&mut value, 0)?;
    let mut canonical = Vec::new();
    ciborium::into_writer(&value, &mut canonical).map_err(|_| Error::InvalidMessage)?;
    if canonical != bytes {
        return Err(Error::NonCanonical);
    }
    let decoded = value.deserialized().map_err(|_| Error::InvalidMessage)?;
    // Also reject omitted fields/defaults or alternate representations that the
    // typed decoder would otherwise silently normalize.
    if encode(&decoded, max_bytes)? != bytes {
        return Err(Error::NonCanonical);
    }
    Ok(decoded)
}

fn normalize(value: &mut Value, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::ResourceExhausted);
    }
    match value {
        Value::Map(entries) => {
            let mut sorted = Vec::with_capacity(entries.len());
            for (key, mut value) in std::mem::take(entries) {
                if !matches!(key, Value::Text(_)) {
                    return Err(Error::InvalidMessage);
                }
                normalize(&mut value, depth + 1)?;
                let mut encoded_key = Vec::new();
                ciborium::into_writer(&key, &mut encoded_key).map_err(|_| Error::InvalidMessage)?;
                sorted.push((encoded_key, key, value));
            }
            // RFC 8949 core deterministic bytewise lexical ordering.
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(Error::InvalidMessage);
            }
            *entries = sorted.into_iter().map(|(_, k, v)| (k, v)).collect();
        }
        Value::Array(values) => {
            for value in values {
                normalize(value, depth + 1)?;
            }
        }
        Value::Integer(number) if u64::try_from(*number).is_ok() => {}
        Value::Bytes(_) | Value::Text(_) | Value::Bool(_) | Value::Null => {}
        _ => return Err(Error::InvalidMessage),
    }
    Ok(())
}

fn preflight(bytes: &[u8], max_bytes: usize) -> Result<()> {
    if bytes.len() > max_bytes {
        return Err(Error::ResourceExhausted);
    }
    let mut reader = Scanner {
        bytes,
        offset: 0,
        items: 0,
    };
    reader.item(0)?;
    if reader.offset != bytes.len() {
        return Err(Error::InvalidMessage);
    }
    Ok(())
}

struct Scanner<'a> {
    bytes: &'a [u8],
    offset: usize,
    items: usize,
}

impl Scanner<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(Error::ResourceExhausted)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(Error::InvalidMessage)?;
        self.offset = end;
        Ok(bytes)
    }

    fn item(&mut self, depth: usize) -> Result<()> {
        self.items += 1;
        if depth > MAX_DEPTH || self.items > MAX_ITEMS {
            return Err(Error::ResourceExhausted);
        }
        let head = self.take(1)?[0];
        let major = head >> 5;
        let additional = head & 31;
        if major == 7 {
            return if matches!(additional, 20..=22) {
                Ok(())
            } else {
                Err(Error::InvalidMessage)
            };
        }
        if !matches!(major, 0 | 2..=5) {
            return Err(Error::InvalidMessage);
        }
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?
                    .try_into()
                    .map_err(|_| Error::InvalidMessage)?,
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?
                    .try_into()
                    .map_err(|_| Error::InvalidMessage)?,
            )),
            27 => u64::from_be_bytes(
                self.take(8)?
                    .try_into()
                    .map_err(|_| Error::InvalidMessage)?,
            ),
            _ => return Err(Error::InvalidMessage),
        };
        match major {
            0 => {}
            2 | 3 => {
                self.take(usize::try_from(value).map_err(|_| Error::ResourceExhausted)?)?;
            }
            4 | 5 => {
                let count = value
                    .checked_mul(if major == 5 { 2 } else { 1 })
                    .ok_or(Error::ResourceExhausted)?;
                if count > (MAX_ITEMS - self.items) as u64
                    || count > (self.bytes.len() - self.offset) as u64
                {
                    return Err(Error::ResourceExhausted);
                }
                for _ in 0..count {
                    self.item(depth + 1)?;
                }
            }
            _ => return Err(Error::InvalidMessage),
        }
        Ok(())
    }
}
