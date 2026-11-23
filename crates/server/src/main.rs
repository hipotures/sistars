use std::net::SocketAddr;
use std::path::PathBuf;

use sistars_server::{build_router, AppState};

#[tokio::main]
async fn main() {
    let bind_addr = std::env::var("SISTARS_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let admin_token = std::env::var("SISTARS_ADMIN_TOKEN").ok();
    let public_url = std::env::var("SISTARS_PUBLIC_URL").ok();
    let mcp_seats_raw = std::env::var("SISTARS_MCP_SEATS").ok();
    let logs_dir = std::env::var("SISTARS_LOGS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("logs"));

    let state = AppState::new_with_mcp(
        admin_token.clone(),
        logs_dir,
        public_url.clone(),
        mcp_seats_raw.as_deref(),
    );
    let app = build_router(state.clone());

    let addr: SocketAddr = bind_addr
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8080)));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind SI-Stars Arena server TCP listener");

    let public_base = public_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&format!("http://{addr}"))
        .trim_end_matches('/')
        .to_string();
    println!("SI-Stars Arena Server listening on http://{addr}");
    println!("OpenAPI 3.0 spec: http://{addr}/openapi.json");
    println!("MCP Streamable HTTP (Header Auth): {public_base}/mcp");
    println!("MCP Streamable HTTP (URL Token):   {public_base}/mcp/<token>");
    {
        let seats = state.persistent_seat_tokens.read().await;
        if !seats.is_empty() {
            let mut names: Vec<&String> = seats.values().collect();
            names.sort();
            names.dedup();
            println!(
                "Persistent MCP seats configured ({}): {}",
                names.len(),
                names
                    .into_iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if admin_token.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        println!("Admin auth: ENABLED (SISTARS_ADMIN_TOKEN)");
    } else {
        println!("Admin auth: OPEN (localhost mode)");
    }

    axum::serve(listener, app)
        .await
        .expect("SI-Stars Arena server exited with error");
}
