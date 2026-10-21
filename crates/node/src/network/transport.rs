// libp2p derives unreachable conversions for the uninhabited connection-limit event.
#![allow(unreachable_code)]
use super::Error;
use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, SwarmBuilder, gossipsub, identify, noise, ping, relay,
    request_response as rr,
    swarm::{NetworkBehaviour, SwarmEvent},
    yamux,
};
use red_leaves_protocol::{
    NetworkScope,
    delivery::{MAX_MESSAGE, Notice, PROTOCOL, Request, Response},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "Event")]
struct Behaviour {
    messages: rr::cbor::Behaviour<Request, Response>,
    gossip: gossipsub::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay_client: relay::client::Behaviour,
    relay_server: libp2p::swarm::behaviour::toggle::Toggle<relay::Behaviour>,
    limits: libp2p::connection_limits::Behaviour,
}
enum Event {
    Messages(rr::Event<Request, Response>),
    Gossip(gossipsub::Event),
    Other,
}
impl From<rr::Event<Request, Response>> for Event {
    fn from(v: rr::Event<Request, Response>) -> Self {
        Self::Messages(v)
    }
}
impl From<gossipsub::Event> for Event {
    fn from(v: gossipsub::Event) -> Self {
        Self::Gossip(v)
    }
}
impl From<identify::Event> for Event {
    fn from(_: identify::Event) -> Self {
        Self::Other
    }
}
impl From<ping::Event> for Event {
    fn from(_: ping::Event) -> Self {
        Self::Other
    }
}
impl From<relay::client::Event> for Event {
    fn from(_: relay::client::Event) -> Self {
        Self::Other
    }
}
impl From<relay::Event> for Event {
    fn from(_: relay::Event) -> Self {
        Self::Other
    }
}
impl From<std::convert::Infallible> for Event {
    fn from(v: std::convert::Infallible) -> Self {
        match v {}
    }
}
#[derive(Default)]
pub struct Status {
    pub running: AtomicBool,
    pub connected: AtomicUsize,
    pub listeners: Mutex<Vec<String>>,
}
pub struct Config {
    pub listen: Multiaddr,
    pub advertise: Vec<Multiaddr>,
    pub peers: Vec<Multiaddr>,
    pub relays: Vec<Multiaddr>,
    pub relay_server: bool,
    pub relay_max_bytes: u64,
}
impl Config {
    pub fn from_env() -> Result<Self, Error> {
        fn list(name: &str) -> Result<Vec<Multiaddr>, Error> {
            let raw = std::env::var(name).unwrap_or_default();
            let values = raw
                .split([',', ' ', '\n'])
                .filter(|s| !s.is_empty())
                .map(str::parse::<Multiaddr>)
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() > 16 {
                return Err("Too many configured P2P peers".into());
            }
            for address in &values {
                if !matches!(
                    address.iter().last(),
                    Some(libp2p::multiaddr::Protocol::P2p(_))
                ) {
                    return Err("P2P peer addresses must end in /p2p/<PeerId>".into());
                }
            }
            Ok(values)
        }
        let relay_max_bytes = std::env::var("RED_LEAVES_RELAY_MAX_BYTES")
            .unwrap_or_else(|_| "134217728".into())
            .parse::<u64>()?;
        if !(1048576..=1073741824).contains(&relay_max_bytes) {
            return Err("Relay byte limit must be between 1 MiB and 1 GiB".into());
        }
        Ok(Self {
            relay_max_bytes,
            advertise: std::env::var("RED_LEAVES_P2P_ADVERTISE")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse())
                .collect::<Result<Vec<_>, _>>()?,
            listen: std::env::var("RED_LEAVES_P2P_LISTEN")
                .unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/4001".into())
                .parse()?,
            peers: list("RED_LEAVES_P2P_PEERS")?,
            relays: list("RED_LEAVES_P2P_RELAYS")?,
            relay_server: std::env::var("RED_LEAVES_P2P_RELAY_SERVER").is_ok_and(|v| v == "true"),
        })
    }
}
enum Command {
    Request {
        peer: PeerId,
        request: Request,
        response: oneshot::Sender<Result<Response, String>>,
    },
    Publish {
        notice: Notice,
        result: oneshot::Sender<Result<(), String>>,
    },
    Reply {
        channel: rr::ResponseChannel<Response>,
        response: Response,
    },
}
pub enum Incoming {
    Request {
        peer: PeerId,
        request: Request,
        reply: oneshot::Sender<Response>,
    },
    Broadcast {
        source: PeerId,
        notice: Notice,
    },
    Connected(PeerId),
}
#[derive(Clone)]
pub struct Handle {
    commands: mpsc::Sender<Command>,
    pub status: Arc<Status>,
}
impl Handle {
    pub async fn request(&self, peer: PeerId, request: Request) -> Result<Response, Error> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .try_send(Command::Request {
                peer,
                request,
                response: tx,
            })
            .map_err(|_| "P2P request queue is full or stopped")?;
        tokio::time::timeout(Duration::from_secs(45), rx)
            .await?
            .map_err(|_| "P2P request canceled")?
            .map_err(Into::into)
    }
    pub async fn publish(&self, notice: Notice) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .try_send(Command::Publish { notice, result: tx })
            .map_err(|_| "P2P command queue is full or stopped")?;
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await?
            .map_err(|_| "P2P publish canceled")?
            .map_err(Into::into)
    }
}
pub fn start(
    key: libp2p::identity::Keypair,
    scope: &NetworkScope,
    config: Config,
) -> Result<
    (
        Handle,
        mpsc::Receiver<Incoming>,
        tokio::task::JoinHandle<()>,
    ),
    Error,
> {
    let topic = gossipsub::IdentTopic::new(format!(
        "red-leaves/public/1/{}/{}",
        scope.chain_id,
        hex::encode(scope.registry_contract.0)
    ));
    let gossip_config = gossipsub::ConfigBuilder::default()
        .max_transmit_size(8192)
        .validation_mode(gossipsub::ValidationMode::Strict)
        .heartbeat_interval(Duration::from_secs(1))
        .build()?;
    let mut gossip = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(key.clone()),
        gossip_config,
    )
    .map_err(std::io::Error::other)?;
    gossip.subscribe(&topic)?;
    let codec = rr::cbor::codec::Codec::default()
        .set_request_size_maximum(MAX_MESSAGE as u64)
        .set_response_size_maximum(MAX_MESSAGE as u64);
    let messages = rr::Behaviour::with_codec(
        codec,
        [(
            StreamProtocol::try_from_owned(format!(
                "{PROTOCOL}/{}/{}",
                scope.chain_id,
                hex::encode(scope.registry_contract.0)
            ))?,
            rr::ProtocolSupport::Full,
        )],
        rr::Config::default()
            .with_request_timeout(Duration::from_secs(40))
            .with_max_concurrent_streams(16),
    );
    let local = key.public().to_peer_id();
    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            Default::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_dns()?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key, relay_client| Behaviour {
            messages,
            gossip,
            identify: identify::Behaviour::new(identify::Config::new(
                PROTOCOL.into(),
                key.public(),
            )),
            ping: ping::Behaviour::default(),
            relay_client,
            relay_server: config
                .relay_server
                .then(|| {
                    relay::Behaviour::new(
                        local,
                        relay::Config {
                            max_circuit_bytes: config.relay_max_bytes,
                            max_circuit_duration: Duration::from_secs(300),
                            ..Default::default()
                        },
                    )
                })
                .into(),
            limits: libp2p::connection_limits::Behaviour::new(
                libp2p::connection_limits::ConnectionLimits::default()
                    .with_max_established(Some(64))
                    .with_max_pending_incoming(Some(16))
                    .with_max_pending_outgoing(Some(16))
                    .with_max_established_per_peer(Some(2)),
            ),
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    swarm.listen_on(config.listen)?;
    for address in &config.advertise {
        swarm.add_external_address(address.clone());
    }
    let addresses = config
        .peers
        .iter()
        .chain(&config.relays)
        .cloned()
        .collect::<Vec<_>>();
    for address in &addresses {
        if let Some(libp2p::multiaddr::Protocol::P2p(peer)) = address.iter().last() {
            swarm.add_peer_address(peer, address.clone());
            let _ = swarm.dial(address.clone());
            swarm.behaviour_mut().gossip.add_explicit_peer(&peer);
        }
    }
    let mut reservations = HashMap::new();
    for relay in &config.relays {
        if let Ok(id) = swarm.listen_on(relay.clone().with(libp2p::multiaddr::Protocol::P2pCircuit))
        {
            reservations.insert(id, relay.clone());
        }
    }
    let (commands, mut rx) = mpsc::channel(64);
    let (incoming, events) = mpsc::channel(64);
    let status = Arc::new(Status::default());
    let handle = Handle {
        commands: commands.clone(),
        status: status.clone(),
    };
    let task = tokio::spawn(async move {
        struct Running(Arc<Status>);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.running.store(false, Ordering::Relaxed);
            }
        }
        let _running = Running(status.clone());
        status.running.store(true, Ordering::Relaxed);
        let mut pending = HashMap::new();
        let mut reconnect = tokio::time::interval(Duration::from_secs(20));
        loop {
            tokio::select! {
                Some(command)=rx.recv()=>match command {
                    Command::Request{peer,request,response}=>{
                        if pending.len()>=32 {let _=response.send(Err("Too many outbound P2P requests".into()));continue;}
                        // A common configured relay makes a registry PeerId dialable behind NAT.
                        for relay in &config.relays {swarm.add_peer_address(peer,relay.clone().with(libp2p::multiaddr::Protocol::P2pCircuit).with(libp2p::multiaddr::Protocol::P2p(peer)));}
                        let id=swarm.behaviour_mut().messages.send_request(&peer,request);pending.insert(id,response);
                    }
                    Command::Publish{notice,result}=>{
                        let sent=serde_json::to_vec(&notice).map_err(|_|"Invalid public notice".to_owned()).and_then(|bytes|swarm.behaviour_mut().gossip.publish(topic.clone(),bytes).map(|_|()).map_err(|_|"No public broadcast subscribers are connected".to_owned()));
                        let _=result.send(sent);
                    }
                    Command::Reply{channel,response}=>{let _=swarm.behaviour_mut().messages.send_response(channel,response);}
                },
                _=reconnect.tick()=>{
                    for address in &addresses {if let Some(libp2p::multiaddr::Protocol::P2p(peer))=address.iter().last()&& !swarm.is_connected(&peer){let _=swarm.dial(address.clone());}}
                    for relay in &config.relays{if !reservations.values().any(|r|r==relay)&& let Ok(id)=swarm.listen_on(relay.clone().with(libp2p::multiaddr::Protocol::P2pCircuit)){reservations.insert(id,relay.clone());}}
                },
                event=swarm.select_next_some()=>match event {
                    SwarmEvent::NewListenAddr{address,..}=>{
                        if config.relay_server && config.advertise.is_empty(){swarm.add_external_address(address.clone());}
                        if let Ok(mut list)=status.listeners.lock(){let value=address.with(libp2p::multiaddr::Protocol::P2p(local)).to_string();if !list.contains(&value) && list.len()<16 {tracing::info!(listen_address=%value,"P2P listener ready");list.push(value);}}
                    }
                    SwarmEvent::ListenerClosed{listener_id,..}=>{reservations.remove(&listener_id);}
                    SwarmEvent::ListenerError{error,..}=>{tracing::warn!(error_kind=?error.kind(),"P2P listener failed");}
                    SwarmEvent::OutgoingConnectionError{error,..}=>{tracing::warn!(error_kind=dial_error_kind(&error),"P2P connection failed");}
                    SwarmEvent::ConnectionEstablished{peer_id,..}=>{tracing::debug!(peer_id=%peer_id,"P2P connection established");status.connected.store(swarm.connected_peers().count(),Ordering::Relaxed);let _=incoming.try_send(Incoming::Connected(peer_id));}
                    SwarmEvent::ConnectionClosed{peer_id,..}=>{tracing::debug!(peer_id=%peer_id,"P2P connection closed");status.connected.store(swarm.connected_peers().count(),Ordering::Relaxed);}
                    SwarmEvent::Behaviour(Event::Messages(rr::Event::Message{peer,message,..}))=>match message {
                        rr::Message::Request{request,channel,..}=>{
                            let (tx,answer)=oneshot::channel();
                            if incoming.try_send(Incoming::Request{peer,request,reply:tx}).is_err(){let _=swarm.behaviour_mut().messages.send_response(channel,Response::Error{code:"busy".into()});}
                            else {let sender=commands.clone();tokio::spawn(async move{let response=tokio::time::timeout(Duration::from_secs(35),answer).await.ok().and_then(Result::ok).unwrap_or(Response::Error{code:"timeout".into()});let _=sender.send(Command::Reply{channel,response}).await;});}
                        }
                        rr::Message::Response{request_id,response}=>{if let Some(tx)=pending.remove(&request_id){let _=tx.send(Ok(response));}}
                    },
                    SwarmEvent::Behaviour(Event::Messages(rr::Event::OutboundFailure{request_id,..}))=>{if let Some(tx)=pending.remove(&request_id){let _=tx.send(Err("P2P peer is offline or unreachable".into()));}},
                    SwarmEvent::Behaviour(Event::Gossip(gossipsub::Event::Message{message,..}))=>{
                        if let (Some(source),Ok(notice))=(message.source,serde_json::from_slice::<Notice>(&message.data)) {let _=incoming.try_send(Incoming::Broadcast{source,notice});}
                    }
                    _=>{}
                }
            }
        }
    });
    Ok((handle, events, task))
}

fn dial_error_kind(error: &libp2p::swarm::DialError) -> &'static str {
    match error {
        libp2p::swarm::DialError::NoAddresses => "no_addresses",
        libp2p::swarm::DialError::WrongPeerId { .. } => "wrong_peer_id",
        libp2p::swarm::DialError::LocalPeerId { .. } => "local_peer_id",
        libp2p::swarm::DialError::Transport(..) => "transport_failure",
        _ => "connection_failed",
    }
}
