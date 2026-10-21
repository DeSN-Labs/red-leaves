#[cfg(feature = "nas")]
mod api;
mod chain;
#[cfg(feature = "nas")]
mod identity;
mod logging;
pub mod network;
#[cfg(feature = "nas")]
mod posts;
mod storage;

#[cfg(any(feature = "nas", feature = "relay"))]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    match dotenvy::from_filename(".env.profile") {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            eprintln!("Unable to load node .env.profile configuration");
            return std::process::ExitCode::FAILURE;
        }
    }
    let role = if cfg!(feature = "nas") {
        "nas"
    } else {
        "relay"
    };
    let _logs = match logging::init(role) {
        Ok(logs) => logs,
        Err(error) => {
            // Before a subscriber exists, only the logger's sanitized setup errors reach stderr.
            eprintln!("Logging initialization failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        node_role = role,
        version = env!("CARGO_PKG_VERSION"),
        "Starting Red Leaves node"
    );
    #[cfg(feature = "nas")]
    let result = run_nas().await;
    #[cfg(not(feature = "nas"))]
    let result = run_relay().await;
    let code = match result {
        Ok(()) => {
            tracing::info!("Node stopped");
            std::process::ExitCode::SUCCESS
        }
        Err(failure) => {
            failure.report();
            std::process::ExitCode::FAILURE
        }
    };
    // WorkerGuard drains the bounded file queue before main returns, including failure exits.
    code
}

#[cfg(feature = "nas")]
async fn run_nas() -> Result<(), logging::Failure> {
    use logging::Failure;
    let config = api::Config::from_env().map_err(Failure::at("nas_configuration"))?;
    let pool = storage::open(
        &std::env::var("RED_LEAVES_DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://red-leaves.sqlite".into()),
    )
    .await
    .map_err(Failure::at("nas_database"))?;
    tracing::info!("NAS storage initialized");
    let enabled = std::env::var("RED_LEAVES_P2P_ENABLED").map_or(true, |v| v != "false");
    let (net, network_task) = if enabled {
        if let Some(service) = &config.posts {
            if service.chain.is_some() {
                let key = network::load_or_create_key(std::path::Path::new(
                    &std::env::var("RED_LEAVES_NODE_KEY_PATH")
                        .unwrap_or_else(|_| "runtime/nas/node.key".into()),
                ))
                .map_err(Failure::at("node_identity"))?;
                let p2p = network::transport::Config::from_env()
                    .map_err(Failure::at("p2p_configuration"))?;
                let scope = service.scope().map_err(Failure::at("p2p_scope"))?;
                let (handle, incoming, task) = network::transport::start(key, &scope, p2p)
                    .map_err(Failure::at("p2p_start"))?;
                (Some((handle, incoming)), Some(task))
            } else {
                tracing::info!("P2P delivery is disabled until a user registry is configured");
                (None, None)
            }
        } else {
            (None, None)
        }
    } else {
        tracing::info!("P2P delivery is disabled by configuration");
        (None, None)
    };
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(Failure::at("api_listener"))?;
    let address = listener.local_addr().map_err(Failure::at("api_listener"))?;
    let delivery = api::start_delivery(pool.clone(), config.clone(), net);
    tracing::info!(listen_address=%address, "Red Leaves NAS API listening");
    let cleanup_pool = pool.clone();
    let cleanup = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if api::cleanup(&cleanup_pool).await.is_err() {
                tracing::error!("Authentication cleanup failed");
            }
        }
    });
    let shutdown_service = config.posts.clone();
    let result = axum::serve(listener, api::router(pool, config))
        .with_graceful_shutdown(async move {
            if shutdown_signal().await.is_err() {
                tracing::error!("Unable to wait for shutdown signal");
            }
            tracing::info!("Shutdown requested");
            if let Some(service) = shutdown_service {
                service
                    .runtime
                    .stopping
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                service.runtime.shutdown.notify_waiters();
            }
        })
        .await;
    cleanup.abort();
    delivery.abort();
    if let Some(task) = network_task {
        task.abort();
    }
    result.map_err(Failure::at("http_server"))?;
    Ok(())
}

#[cfg(all(not(feature = "nas"), feature = "relay"))]
async fn run_relay() -> Result<(), logging::Failure> {
    use logging::Failure;
    let address = std::env::var("RED_LEAVES_USER_REGISTRY_ADDRESS")
        .map_err(Failure::at("relay_registry_configuration"))?;
    let raw = address
        .strip_prefix("0x")
        .ok_or_else(|| Failure::at("relay_registry_configuration")("Invalid registry address"))?;
    let decoded = hex::decode(raw).map_err(Failure::at("relay_registry_configuration"))?;
    let registry = red_leaves_protocol::Address::from_slice(&decoded)
        .map_err(Failure::at("relay_registry_configuration"))?;
    let scope = red_leaves_protocol::NetworkScope::new(84532, registry)
        .map_err(Failure::at("p2p_scope"))?;
    let key = network::load_or_create_key(std::path::Path::new(
        &std::env::var("RED_LEAVES_NODE_KEY_PATH")
            .unwrap_or_else(|_| "runtime/relay/node.key".into()),
    ))
    .map_err(Failure::at("node_identity"))?;
    let mut config =
        network::transport::Config::from_env().map_err(Failure::at("p2p_configuration"))?;
    config.relay_server = true;
    let (_, mut incoming, task) =
        network::transport::start(key, &scope, config).map_err(Failure::at("p2p_start"))?;
    tracing::info!(
        "Red Leaves relay started; billing and PostgreSQL persistence are not implemented"
    );
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result=&mut shutdown=>{result.map_err(Failure::at("shutdown_signal"))?;break;}
            Some(event)=incoming.recv()=>{if let network::transport::Incoming::Request{reply,..}=event{let _=reply.send(red_leaves_protocol::delivery::Response::Error{code:"relay_only".into()});}}
        }
    }
    task.abort();
    Ok(())
}

#[cfg(any(feature = "nas", feature = "relay"))]
async fn shutdown_signal() -> Result<(), network::Error> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(not(any(feature = "nas", feature = "relay")))]
fn main() -> std::process::ExitCode {
    match logging::init("unconfigured") {
        Ok(_logs) => tracing::error!("Enable the nas or relay node feature"),
        Err(error) => eprintln!("Logging initialization failed: {error}"),
    }
    std::process::ExitCode::FAILURE
}
