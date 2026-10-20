#[cfg(feature = "nas")]
mod api;
mod chain;
#[cfg(feature = "nas")]
mod identity;
mod network;
mod storage;

#[cfg(feature = "nas")]
#[tokio::main]
async fn main() -> Result<(), identity::Error> {
    match dotenvy::from_filename(".env.profile") {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Unable to load node .env.profile configuration".into()),
    }
    let config = api::Config::from_env()?;
    let pool = storage::open(
        &std::env::var("RED_LEAVES_DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://red-leaves.sqlite".into()),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    println!(
        "Red Leaves NAS authentication API listening on {}",
        listener.local_addr()?
    );
    let cleanup_pool = pool.clone();
    let cleanup = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = api::cleanup(&cleanup_pool).await {
                eprintln!("Authentication cleanup failed: {error}");
            }
        }
    });
    let result = axum::serve(listener, api::router(pool, config))
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("Shutdown signal failed: {error}");
            }
        })
        .await;
    cleanup.abort();
    result?;
    Ok(())
}

#[cfg(not(feature = "nas"))]
fn main() {
    println!(
        "Red Leaves relay scaffold. Networking, APIs and PostgreSQL storage are not implemented yet."
    );
}
