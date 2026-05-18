//! HTTP proxy for osu! web requests.
//!
//! This module provides an HTTP proxy that intercepts osu! client requests and
//! routes them to either the rai.moe beatmap mirror (for osu!direct functionality)
//! or the official osu! servers (for everything else).
//!
//! # Request Routing
//!
//! Requests are routed based on the host header and URL path:
//!
//! - **osu!direct requests** (search, download, thumbnails) -> `direct.rai.moe`
//!   well, now it changes the chat messages `osu.localhost <-> osu.ppy.sh`
//! - **All other requests** (login, scores, multiplayer) -> official `*.ppy.sh` servers
//!
//! This selective routing ensures that only beatmap-related traffic goes through
//! the mirror, while sensitive operations remain on official servers.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};

use crate::domain::domain_rewriter::DomainRewriter;
use crate::domain::{
    inject_supporter_privileges, map_host_to_upstream, route_request, AppState, Packet,
    RouteDecision, ServerPacketId,
};
use crate::infrastructure::tls::create_tls_acceptor;

/// Checks if host is localhost, 127.0.0.1, [::1], or *.localhost (with optional port).
fn is_valid_localhost_host(host: &str) -> bool {
    let host_without_port = if host.starts_with('[') {
        host.find(']').map(|i| &host[..=i]).unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };

    let h = host_without_port.to_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h.ends_with(".localhost")
}

/// Runs the HTTPS proxy server with TLS.
pub async fn run_https_proxy(
    port: u16,
    direct_base_url: &str,
    inject_supporter: bool,
    upstream_server: &str,
    state: Arc<RwLock<AppState>>,
    mut shutdown: oneshot::Receiver<()>,
    ready_tx: Option<oneshot::Sender<()>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tls_acceptor = create_tls_acceptor()?;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "Port {} is already in use. Please close any application using this port.",
                port
            )
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "Permission denied binding to port {}. Try running as Administrator.",
                port
            )
        } else {
            format!("Failed to bind to port {}: {}", port, e)
        };
        tracing::error!("{}", msg);
        msg
    })?;

    tracing::info!("HTTPS proxy listening on {}", addr);

    // Signal that we're ready (port is bound)
    if let Some(tx) = ready_tx {
        let _ = tx.send(());
    }

    let direct_base_url = direct_base_url.to_string();
    let upstream_server = upstream_server.to_string();
    let (connection_shutdown_tx, _) = broadcast::channel::<()>(1);

    // Create a shared HTTP client with connection pooling and timeouts
    let client = Arc::new(
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default(),
    );

    let rewriter = Arc::new(DomainRewriter::new(&upstream_server));

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, client_addr) = result?;

                let tls_acceptor = tls_acceptor.clone();
                let state = Arc::clone(&state);
                let direct_base_url = direct_base_url.clone();
                let upstream_server = upstream_server.clone();
                let client = Arc::clone(&client);
                let rewriter = Arc::clone(&rewriter);
                let mut connection_shutdown_rx = connection_shutdown_tx.subscribe();

                tokio::spawn(async move {
                    let tls_stream = match tls_acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::debug!("TLS handshake failed from {}: {}", client_addr, e);
                            return;
                        }
                    };

                    let io = TokioIo::new(tls_stream);

                    let service = service_fn(move |req| {
                        handle_request(
                            req,
                            direct_base_url.clone(),
                            inject_supporter,
                            upstream_server.clone(),
                            Arc::clone(&state),
                            Arc::clone(&client),
                            Arc::clone(&rewriter)
                        )
                    });

                    let connection = http1::Builder::new()
                        .keep_alive(false)
                        .serve_connection(io, service);

                    tokio::select! {
                        result = connection => {
                            if let Err(err) = result {
                                tracing::debug!("Connection error from {}: {:?}", client_addr, err);
                            }
                        }
                        _ = connection_shutdown_rx.recv() => {
                            tracing::debug!("Closing active connection from {}", client_addr);
                        }
                    }
                });
            }
            _ = &mut shutdown => {
                tracing::info!("HTTPS proxy shutting down");
                let _ = connection_shutdown_tx.send(());
                break;
            }
        }
    }

    Ok(())
}

/// Handles a single HTTP request from the osu! client.
async fn handle_request(
    req: Request<Incoming>,
    direct_base_url: String,
    inject_supporter: bool,
    upstream_server: String,
    state: Arc<RwLock<AppState>>,
    client: Arc<reqwest::Client>,
    rewriter: Arc<DomainRewriter>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
        .to_string();

    if !is_valid_localhost_host(&host) {
        tracing::warn!(
            "Rejected request with invalid host header: {} (expected localhost)",
            host
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "Invalid host header: only localhost connections are allowed",
        ));
    }

    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    tracing::debug!("Request: {} {} (host: {})", req.method(), path, &host);

    let decision = route_request(&host, path);

    {
        let mut s = state.write();
        s.requests_proxied += 1;
    }

    let response = match decision {
        RouteDecision::HandleLocally => {
            if path.starts_with("/d/") {
                let mut s = state.write();
                s.beatmaps_downloaded += 1;
            }
            forward_to_raimoe(req, &direct_base_url, &client, &rewriter).await
        }
        RouteDecision::ForwardToUpstream => {
            forward_to_upstream(
                req,
                &host,
                inject_supporter,
                &upstream_server,
                &client,
                &rewriter,
            )
            .await
        }
        RouteDecision::RedirectToUpstream => {
            let upstream_host = map_host_to_upstream(&host, &upstream_server);
            let redirect_url = format!("https://{}{}", upstream_host, path);
            tracing::debug!("Redirecting to: {}", redirect_url);
            redirect_response(&redirect_url)
        }
    };

    Ok(response)
}

/// Forwards a request to the rai.moe beatmap mirror.
async fn forward_to_raimoe(
    req: Request<Incoming>,
    direct_base_url: &str,
    client: &reqwest::Client,
    rewriter: &DomainRewriter,
) -> Response<BoxBody<Bytes, Infallible>> {
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", direct_base_url.trim_end_matches('/'), path);

    tracing::debug!("Forwarding to rai.moe: {}", url);

    match forward_request(req, &url, client, rewriter).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to forward to rai.moe: {}", e);
            error_response(StatusCode::BAD_GATEWAY, "Failed to reach rai.moe")
        }
    }
}

async fn forward_to_upstream(
    req: Request<Incoming>,
    host: &str,
    inject_supporter: bool,
    upstream_server: &str,
    client: &reqwest::Client,
    rewriter: &DomainRewriter,
) -> Response<BoxBody<Bytes, Infallible>> {
    let upstream_host = map_host_to_upstream(host, upstream_server);
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("https://{}{}", upstream_host, path);

    tracing::debug!("Forwarding to {}: {}", upstream_server, url);

    let is_bancho = upstream_host.starts_with("c.");

    match forward_request_with_injection(req, &url, client, inject_supporter && is_bancho, rewriter)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to forward to {}: {}", upstream_server, e);
            error_response(StatusCode::BAD_GATEWAY, "Failed to reach osu! servers")
        }
    }
}

async fn forward_request(
    req: Request<Incoming>,
    url: &str,
    client: &reqwest::Client,
    rewriter: &DomainRewriter,
) -> Result<Response<BoxBody<Bytes, Infallible>>, reqwest::Error> {
    forward_request_with_injection(req, url, client, false, rewriter).await
}

async fn forward_request_with_injection(
    req: Request<Incoming>,
    url: &str,
    client: &reqwest::Client,
    inject_supporter: bool,
    rewriter: &DomainRewriter,
) -> Result<Response<BoxBody<Bytes, Infallible>>, reqwest::Error> {
    let method = match *req.method() {
        Method::POST => reqwest::Method::POST,
        Method::GET => reqwest::Method::GET,
        _ => reqwest::Method::GET,
    };

    let mut builder = client.request(method, url);

    for (name, value) in req.headers() {
        let name_s = name.as_str().to_lowercase();
        if !matches!(name_s.as_str(), "host" | "connection" | "content-length") {
            builder = builder.header(name, value);
        }
    }

    let body_data = req
        .collect()
        .await
        .ok()
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let final_req_body = if !body_data.is_empty() {
        rewriter.rewrite_forward(body_data)
    } else {
        body_data
    };

    let resp = builder.body(final_req_body).send().await?;

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut response_builder = Response::builder().status(status);

    for (name, value) in resp.headers() {
        let name_str = name.as_str();
        if !matches!(
            name_str.to_lowercase().as_str(),
            "transfer-encoding" | "connection" | "content-length"
        ) {
            if let Ok(v) = value.to_str() {
                response_builder = response_builder.header(name_str, v);
            }
        }
    }

    let mut body_bytes = resp.bytes().await.unwrap_or_default();

    // If supporter injection is enabled, parse and modify Bancho packets
    if inject_supporter && !body_bytes.is_empty() {
        body_bytes = inject_supporter_into_bancho_response(body_bytes);
    }

    if !body_bytes.is_empty() {
        body_bytes = rewriter.rewrite_backward(body_bytes);
    }

    let body = Full::new(body_bytes).map_err(|_| unreachable!()).boxed();

    Ok(response_builder.body(body).unwrap())
}

/// Parses Bancho packets from the response body and injects supporter
/// privileges into any UserPrivileges packets.
fn inject_supporter_into_bancho_response(body: Bytes) -> Bytes {
    let (mut packets, remaining) = Packet::parse_stream(&body);

    if packets.is_empty() && remaining.is_empty() {
        // No valid packets found, return original
        return body;
    }

    let mut modified = false;

    for packet in &mut packets {
        let packet_id = packet.header.packet_id;
        let is_chat = matches!(packet_id, 0 | 7);
        let mut packet_modified = false;

        if packet.packet_type() == ServerPacketId::UserPrivileges {
            tracing::debug!("Injecting supporter privileges into UserPrivileges packet");
            inject_supporter_privileges(packet);
            packet_modified = true;
        }

        if packet_modified {
            modified = true;
        } else if is_chat {
            tracing::info!(
                "Processing incoming chat packet (ID: {}), payload size: {}",
                packet_id,
                packet.payload.len()
            );
        }
    }

    if !modified {
        // No modifications needed, return original
        return body;
    }

    // Reassemble packets into response body
    let mut output = Vec::new();
    for packet in packets {
        output.extend(packet.to_bytes());
    }
    // Append any remaining unparsed data (incomplete packets)
    output.extend(remaining);

    Bytes::from(output)
}

/// Creates an error response with the given status code and message.
///
/// Used for returning error responses when upstream requests fail.
///
/// # Arguments
///
/// * `status` - The HTTP status code (typically 502 Bad Gateway)
/// * `message` - Human-readable error message
///
/// # Returns
///
/// An HTTP response with the specified status and plain text body.
fn error_response(status: StatusCode, message: &str) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(
            Full::new(Bytes::from(message.to_string()))
                .map_err(|_| unreachable!())
                .boxed(),
        )
        .unwrap()
}

/// Creates a redirect response to the given URL.
///
/// Returns a 302 Found response that redirects the browser to the target URL.
/// Used for redirecting website requests to osu.ppy.sh.
fn redirect_response(url: &str) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header("location", url)
        .body(Full::new(Bytes::new()).map_err(|_| unreachable!()).boxed())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_localhost_valid() {
        assert!(is_valid_localhost_host("localhost"));
        assert!(is_valid_localhost_host("LOCALHOST"));
        assert!(is_valid_localhost_host("LocalHost"));
    }

    #[test]
    fn test_localhost_with_port_valid() {
        assert!(is_valid_localhost_host("localhost:80"));
        assert!(is_valid_localhost_host("localhost:443"));
        assert!(is_valid_localhost_host("localhost:8080"));
    }

    #[test]
    fn test_ipv4_localhost_valid() {
        assert!(is_valid_localhost_host("127.0.0.1"));
        assert!(is_valid_localhost_host("127.0.0.1:80"));
        assert!(is_valid_localhost_host("127.0.0.1:443"));
    }

    #[test]
    fn test_ipv6_localhost_valid() {
        assert!(is_valid_localhost_host("[::1]"));
        assert!(is_valid_localhost_host("[::1]:80"));
        assert!(is_valid_localhost_host("[::1]:443"));
    }

    #[test]
    fn test_localhost_subdomains_valid() {
        assert!(is_valid_localhost_host("osu.localhost"));
        assert!(is_valid_localhost_host("c.localhost"));
        assert!(is_valid_localhost_host("a.localhost"));
        assert!(is_valid_localhost_host("b.localhost"));
        assert!(is_valid_localhost_host("sub.domain.localhost"));
        assert!(is_valid_localhost_host("osu.localhost:80"));
        assert!(is_valid_localhost_host("c.localhost:443"));
    }

    #[test]
    fn test_external_hosts_rejected() {
        assert!(!is_valid_localhost_host("example.com"));
        assert!(!is_valid_localhost_host("osu.ppy.sh"));
        assert!(!is_valid_localhost_host("evil.com"));
        assert!(!is_valid_localhost_host("google.com:443"));
    }

    #[test]
    fn test_malicious_hosts_rejected() {
        assert!(!is_valid_localhost_host("localhost.evil.com"));
        assert!(!is_valid_localhost_host("127.0.0.1.evil.com"));
        assert!(!is_valid_localhost_host("notlocalhost"));
        assert!(!is_valid_localhost_host("localhost@evil.com"));
        assert!(!is_valid_localhost_host("evil.com:localhost"));
    }

    #[test]
    fn test_other_loopback_addresses_rejected() {
        assert!(!is_valid_localhost_host("127.0.0.2"));
        assert!(!is_valid_localhost_host("127.1.1.1"));
    }

    #[test]
    fn test_private_networks_rejected() {
        assert!(!is_valid_localhost_host("192.168.1.1"));
        assert!(!is_valid_localhost_host("10.0.0.1"));
        assert!(!is_valid_localhost_host("172.16.0.1"));
    }

    #[test]
    fn test_empty_host_rejected() {
        assert!(!is_valid_localhost_host(""));
    }

    #[test]
    fn test_ipv6_malformed_rejected() {
        assert!(!is_valid_localhost_host("::1"));
        assert!(!is_valid_localhost_host("[::2]"));
    }
}
