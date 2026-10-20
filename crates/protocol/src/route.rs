//! Signed, short-lived relay routes. Discovery and DNS resolution are node concerns.

use std::net::IpAddr;

use libp2p_core::SignedEnvelope;
use libp2p_identity::{Keypair, PeerId};
use multiaddr::{Multiaddr, Protocol};
use serde::{Deserialize, Serialize};

use crate::{ContentId, Error, MAX_ROUTE, NetworkScope, Result, canonical};

pub const ROUTE_DOMAIN: &str = "red-leaves/nas-route/1";
pub const ROUTE_PAYLOAD_TYPE: &[u8] = b"/red-leaves/nas-route/1";
pub const MAX_ROUTE_LIFETIME: u64 = 600;
pub const CLOCK_SKEW: u64 = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRoute {
    #[serde(with = "serde_bytes")]
    pub relay_peer_id: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub circuit_multiaddr: Vec<u8>,
    pub reservation_expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRecord {
    pub scope: NetworkScope,
    #[serde(with = "serde_bytes")]
    pub subject_peer_id: Vec<u8>,
    pub revision: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub relay_routes: Vec<RelayRoute>,
}

impl RouteRecord {
    pub fn validate(&self, expected_scope: &NetworkScope, now: u64) -> Result<PeerId> {
        self.scope.ensure_matches(expected_scope)?;
        let subject =
            PeerId::from_bytes(&self.subject_peer_id).map_err(|_| Error::InvalidMessage)?;
        if self.revision == 0
            || self.issued_at == 0
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > MAX_ROUTE_LIFETIME
            || self.issued_at > now.saturating_add(CLOCK_SKEW)
        {
            return Err(Error::InvalidMessage);
        }
        if now >= self.expires_at.saturating_add(CLOCK_SKEW) {
            return Err(Error::Expired);
        }
        if self.relay_routes.is_empty() || self.relay_routes.len() > 4 {
            return Err(Error::ResourceExhausted);
        }
        let mut addresses = std::collections::HashSet::new();
        for route in &self.relay_routes {
            let relay =
                PeerId::from_bytes(&route.relay_peer_id).map_err(|_| Error::InvalidMessage)?;
            if relay == subject
                || route.reservation_expires_at < self.expires_at
                || now >= route.reservation_expires_at
                || route.circuit_multiaddr.len() > 1024
            {
                return Err(Error::InvalidMessage);
            }
            let addr = Multiaddr::try_from(route.circuit_multiaddr.clone())
                .map_err(|_| Error::InvalidMessage)?;
            validate_circuit(&addr, relay, subject)?;
            if !addresses.insert(route.circuit_multiaddr.as_slice()) {
                return Err(Error::InvalidMessage);
            }
        }
        Ok(subject)
    }

    pub fn sign(&self, key: &Keypair, now: u64) -> Result<Vec<u8>> {
        let subject = self.validate(&self.scope, now)?;
        if key.public().to_peer_id() != subject {
            return Err(Error::InvalidSignature);
        }
        let payload = canonical::encode(self, MAX_ROUTE)?;
        let envelope = SignedEnvelope::new(
            key,
            ROUTE_DOMAIN.into(),
            ROUTE_PAYLOAD_TYPE.to_vec(),
            payload,
        )
        .map_err(|_| Error::InvalidSignature)?
        .into_protobuf_encoding();
        if envelope.len() > MAX_ROUTE {
            return Err(Error::ResourceExhausted);
        }
        Ok(envelope)
    }
}

pub struct VerifiedRoute {
    record: RouteRecord,
    record_id: ContentId,
}

impl VerifiedRoute {
    pub fn record(&self) -> &RouteRecord {
        &self.record
    }

    /// Check against a checkpoint loaded from durable storage, then persist the
    /// returned checkpoint before using/publishing the new route. No hidden global map.
    pub fn checkpoint(&self, previous: Option<&RouteCheckpoint>) -> Result<RouteCheckpoint> {
        if let Some(previous) = previous {
            self.record.scope.ensure_matches(&previous.scope)?;
            if self.record.subject_peer_id != previous.subject_peer_id {
                return Err(Error::InvalidMessage);
            }
            if self.record.revision < previous.revision {
                return Err(Error::Replay);
            }
            if self.record.revision == previous.revision && self.record_id != previous.record_id {
                return Err(Error::Conflict);
            }
        }
        Ok(RouteCheckpoint {
            scope: self.record.scope.clone(),
            subject_peer_id: self.record.subject_peer_id.clone(),
            revision: self.record.revision,
            record_id: self.record_id,
            retain_until: self
                .record
                .expires_at
                .saturating_add(CLOCK_SKEW)
                .max(previous.map_or(0, |checkpoint| checkpoint.retain_until)),
        })
    }
}

/// Persist per (network scope, subject PeerId); do not discard on disconnect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCheckpoint {
    pub scope: NetworkScope,
    #[serde(with = "serde_bytes")]
    pub subject_peer_id: Vec<u8>,
    pub revision: u64,
    pub record_id: ContentId,
    pub retain_until: u64,
}

pub fn verify(
    envelope: &[u8],
    scope: &NetworkScope,
    expected_peer: PeerId,
    now: u64,
) -> Result<VerifiedRoute> {
    if envelope.is_empty() || envelope.len() > MAX_ROUTE {
        return Err(Error::ResourceExhausted);
    }
    let signed =
        SignedEnvelope::from_protobuf_encoding(envelope).map_err(|_| Error::InvalidMessage)?;
    let (payload, key) = signed
        .payload_and_signing_key(ROUTE_DOMAIN.into(), ROUTE_PAYLOAD_TYPE)
        .map_err(|_| Error::InvalidSignature)?;
    if key.to_peer_id() != expected_peer {
        return Err(Error::InvalidSignature);
    }
    let record: RouteRecord = canonical::decode(payload, MAX_ROUTE)?;
    if record.validate(scope, now)? != expected_peer {
        return Err(Error::InvalidSignature);
    }
    Ok(VerifiedRoute {
        record,
        record_id: ContentId::of(payload),
    })
}

pub fn discovery_key(scope: &NetworkScope, peer: PeerId) -> Result<ContentId> {
    #[derive(Serialize)]
    struct Key<'a> {
        scope: &'a NetworkScope,
        purpose: &'static str,
        #[serde(with = "serde_bytes")]
        peer: Vec<u8>,
    }
    scope.validate()?;
    Ok(ContentId::of(&canonical::encode(
        &Key {
            scope,
            purpose: "nas-route",
            peer: peer.to_bytes(),
        },
        MAX_ROUTE,
    )?))
}

fn validate_circuit(address: &Multiaddr, relay: PeerId, subject: PeerId) -> Result<()> {
    let mut parts = address.iter();
    match parts.next() {
        Some(Protocol::Ip4(ip)) if is_public_ip(IpAddr::V4(ip)) => {}
        Some(Protocol::Ip6(ip)) if is_public_ip(IpAddr::V6(ip)) => {}
        Some(Protocol::Dns(name) | Protocol::Dns4(name) | Protocol::Dns6(name))
            if public_dns_name(&name) => {}
        _ => return Err(Error::InvalidMessage),
    }
    match parts.next() {
        Some(Protocol::Tcp(port)) if port > 0 => {}
        Some(Protocol::Udp(port)) if port > 0 => {
            if parts.next() != Some(Protocol::QuicV1) {
                return Err(Error::InvalidMessage);
            }
        }
        _ => return Err(Error::InvalidMessage),
    }
    if parts.next() != Some(Protocol::P2p(relay))
        || parts.next() != Some(Protocol::P2pCircuit)
        || parts.next() != Some(Protocol::P2p(subject))
        || parts.next().is_some()
    {
        return Err(Error::InvalidMessage);
    }
    Ok(())
}

/// Apply again to ALL resolved DNS addresses immediately before dialing and reconnecting.
/// This is a conservative public-discovery policy; trusted LAN pairing is separate.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            // Only global-unicast space; exclude documentation, transition and
            // special-purpose ranges, including mapped IPv4 bypasses.
            let segments = ip.segments();
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && segments[1] < 0x200)
                && !(segments[0] == 0x2001 && segments[1] == 0xdb8)
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && (segments[1] & 0xf000) == 0)
        }
    }
}

fn public_dns_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name.len() <= 253
        && name.contains('.')
        && ![".local", ".localhost", ".internal", ".invalid", ".test"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
