//! EOA-only SIWE authentication. No RPC or service authorization is performed.
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use siwe::{Message, VerificationOpts};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const CHAIN_ID: u64 = 84532;
pub const CHALLENGE_SECONDS: i64 = 300;
pub const SESSION_SECONDS: i64 = 3600;
pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub fn secret() -> Result<String, Error> {
    let mut bytes = [0u8; 32];
    OsRng.try_fill_bytes(&mut bytes)?;
    Ok(hex::encode(bytes))
}
pub fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn address(value: &str) -> Result<[u8; 20], Error> {
    let bytes = hex::decode(value.strip_prefix("0x").ok_or("Invalid address")?)?;
    Ok(bytes.try_into().map_err(|_| "Invalid address length")?)
}
pub fn message(
    address: [u8; 20],
    domain: &str,
    uri: &str,
    nonce: String,
) -> Result<Message, Error> {
    let now = OffsetDateTime::now_utc();
    Ok(Message {
        domain: domain.parse()?,
        address,
        statement: Some(
            "Sign in to Red Leaves. This does not authorize transactions or NAS binding.".into(),
        ),
        uri: uri.parse()?,
        version: siwe::Version::V1,
        chain_id: CHAIN_ID,
        nonce,
        issued_at: now.format(&Rfc3339)?.parse()?,
        expiration_time: Some(
            (now + time::Duration::seconds(CHALLENGE_SECONDS))
                .format(&Rfc3339)?
                .parse()?,
        ),
        not_before: None,
        request_id: None,
        resources: vec![],
    })
}
pub async fn verify(raw: &str, signature: &str, domain: &str, uri: &str) -> Result<(), Error> {
    let message: Message = raw.parse()?;
    if message.chain_id != CHAIN_ID || message.uri.as_str() != uri {
        return Err("Invalid signing scope".into());
    }
    let signature = hex::decode(signature.strip_prefix("0x").ok_or("Invalid signature")?)?;
    if signature.len() != 65 {
        return Err("Only EOA signatures are supported".into());
    }
    message
        .verify(
            &signature,
            &VerificationOpts {
                domain: Some(domain.parse()?),
                nonce: Some(message.nonce.clone()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}
