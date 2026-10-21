//! AES-256-GCM bodies and RFC 9180 HPKE key envelopes. No custom key agreement.
use crate::{ContentId, Error, Result};
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use rand09::{RngCore, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

type XKem = hpke::kem::X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type HpkeAead = hpke::aead::ChaCha20Poly1305;
const INFO: &[u8] = b"red-leaves/content-key/1/x25519-hkdf-sha256-chacha20poly1305";

fn rng() -> Result<StdRng> {
    StdRng::try_from_os_rng().map_err(|_| Error::ResourceExhausted)
}
pub fn random_key() -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0; 32]);
    rng()?.fill_bytes(&mut *key);
    Ok(key)
}
pub fn key_pair() -> Result<(Zeroizing<Vec<u8>>, Vec<u8>)> {
    let (secret, public) = XKem::gen_keypair(&mut rng()?);
    Ok((
        Zeroizing::new(secret.to_bytes().to_vec()),
        public.to_bytes().to_vec(),
    ))
}
pub fn public_key(secret: &[u8]) -> Result<Vec<u8>> {
    let key = <XKem as Kem>::PrivateKey::from_bytes(secret).map_err(|_| Error::InvalidMessage)?;
    Ok(XKem::sk_to_pk(&key).to_bytes().to_vec())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sealed {
    #[serde(with = "serde_bytes")]
    pub nonce: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ciphertext: Vec<u8>,
}
pub fn seal(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Sealed> {
    let mut nonce = [0; 12];
    rng()?.fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::InvalidMessage)?;
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| Error::IntegrityFailed)?;
    Ok(Sealed {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}
pub fn open(key: &[u8; 32], sealed: &Sealed, aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let nonce: &[u8; 12] = sealed
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidMessage)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::InvalidMessage)?;
    cipher
        .decrypt(
            nonce.into(),
            Payload {
                msg: &sealed.ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::IntegrityFailed)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEnvelope {
    pub public_key_id: ContentId,
    #[serde(with = "serde_bytes")]
    pub encapsulated_key: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ciphertext: Vec<u8>,
}
pub fn wrap(public: &[u8], key: &[u8; 32], aad: &[u8]) -> Result<KeyEnvelope> {
    let public_key =
        <XKem as Kem>::PublicKey::from_bytes(public).map_err(|_| Error::InvalidMessage)?;
    let (enc, mut context) = hpke::setup_sender::<HpkeAead, Kdf, XKem, _>(
        &OpModeS::Base,
        &public_key,
        INFO,
        &mut rng()?,
    )
    .map_err(|_| Error::IntegrityFailed)?;
    let ciphertext = context.seal(key, aad).map_err(|_| Error::IntegrityFailed)?;
    Ok(KeyEnvelope {
        public_key_id: ContentId::of(public),
        encapsulated_key: enc.to_bytes().to_vec(),
        ciphertext,
    })
}
pub fn unwrap(secret: &[u8], envelope: &KeyEnvelope, aad: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    if ContentId::of(&public_key(secret)?) != envelope.public_key_id {
        return Err(Error::InvalidMessage);
    }
    let secret =
        <XKem as Kem>::PrivateKey::from_bytes(secret).map_err(|_| Error::InvalidMessage)?;
    let enc = <XKem as Kem>::EncappedKey::from_bytes(&envelope.encapsulated_key)
        .map_err(|_| Error::InvalidMessage)?;
    let mut context =
        hpke::setup_receiver::<HpkeAead, Kdf, XKem>(&OpModeR::Base, &secret, &enc, INFO)
            .map_err(|_| Error::IntegrityFailed)?;
    let key = Zeroizing::new(
        context
            .open(&envelope.ciphertext, aad)
            .map_err(|_| Error::IntegrityFailed)?,
    );
    Ok(Zeroizing::new(
        key.as_slice()
            .try_into()
            .map_err(|_| Error::InvalidMessage)?,
    ))
}
