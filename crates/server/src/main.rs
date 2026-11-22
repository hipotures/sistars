use std::net::SocketAddr;
use std::path::PathBuf;

use sistars_server::{build_router, AppState};

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("SISTARS_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let admin_token = std::env::var("SISTARS_ADMIN_TOKEN").ok();
    let logs_dir = std::env::var("SISTARS_LOGS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("logs"));

    let state = AppState::new(admin_token.clone(), logs_dir);
    let app = build_router(state);

    let addr: SocketAddr = bind_addr
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8080)));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind SI-Stars Arena server TCP listener");

    println!("SI-Stars Arena Server listening on http://{addr}");
    println!("OpenAPI 3.0 spec: http://{addr}/openapi.json");
    if admin_token.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        println!("Admin auth: ENABLED (SISTARS_ADMIN_TOKEN)");
    } else {
        println!("Admin auth: OPEN (localhost mode)");
    }

    axum::serve(listener, app)
        .await
        .expect("SI-Stars Arena server exited with error");
}
