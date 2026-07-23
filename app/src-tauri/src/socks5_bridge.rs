use base64::{prelude::BASE64_STANDARD, Engine};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub trait AsyncReadWrite: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

type BoxedStream = Box<dyn AsyncReadWrite>;

/// Starts a local SOCKS5 bridge.
/// Returns the local port it is listening on, and the JoinHandle for the server task.
pub async fn start_bridge(
    upstream_host: String,
    upstream_port: u16,
    upstream_scheme: String,
    upstream_username: String,
    upstream_password: String,
) -> Result<(u16, JoinHandle<()>), String> {
    // Bind to a random port on localhost
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local SOCKS5 bridge: {}", e))?;

    let local_port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {}", e))?
        .port();

    let upstream_host = Arc::new(upstream_host);
    let upstream_scheme = Arc::new(upstream_scheme);
    let upstream_username = Arc::new(upstream_username);
    let upstream_password = Arc::new(upstream_password);

    log::info!(
        "SOCKS5 bridge listening on 127.0.0.1:{} tunneling to {}://{}:{}",
        local_port,
        upstream_scheme,
        upstream_host,
        upstream_port
    );

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((client_stream, client_addr)) => {
                    let upstream_host = upstream_host.clone();
                    let upstream_scheme = upstream_scheme.clone();
                    let upstream_username = upstream_username.clone();
                    let upstream_password = upstream_password.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_client(
                            client_stream,
                            client_addr,
                            &upstream_host,
                            upstream_port,
                            &upstream_scheme,
                            &upstream_username,
                            &upstream_password,
                        )
                        .await
                        {
                            log::debug!("Bridge connection error for {}: {}", client_addr, e);
                        }
                    });
                }
                Err(e) => {
                    log::error!("SOCKS5 bridge accept error: {}", e);
                    break;
                }
            }
        }
    });

    Ok((local_port, handle))
}

async fn handle_client(
    mut client_stream: TcpStream,
    client_addr: SocketAddr,
    upstream_host: &str,
    upstream_port: u16,
    upstream_scheme: &str,
    upstream_username: &str,
    upstream_password: &str,
) -> Result<(), String> {
    // 1. SOCKS5 greeting
    let mut greeting = [0u8; 2];
    client_stream
        .read_exact(&mut greeting)
        .await
        .map_err(|e| format!("Failed reading greeting: {}", e))?;

    if greeting[0] != 0x05 {
        return Err(format!("Unsupported SOCKS version: {}", greeting[0]));
    }

    let num_methods = greeting[1] as usize;
    let mut methods = vec![0u8; num_methods];
    client_stream
        .read_exact(&mut methods)
        .await
        .map_err(|e| format!("Failed reading methods: {}", e))?;

    // We only support No-Auth (0x00) for local connections
    if !methods.contains(&0x00) {
        client_stream.write_all(&[0x05, 0xff]).await.ok(); // No acceptable methods
        return Err("No acceptable auth methods".into());
    }

    // Accept No-Auth
    client_stream
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| format!("Failed writing auth selection: {}", e))?;

    // 2. SOCKS5 Request
    let mut req_header = [0u8; 4];
    client_stream
        .read_exact(&mut req_header)
        .await
        .map_err(|e| format!("Failed reading request header: {}", e))?;

    if req_header[0] != 0x05 {
        return Err("Invalid SOCKS version in request".into());
    }

    if req_header[1] != 0x01 {
        // We only support CONNECT (0x01)
        client_stream
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .ok();
        return Err(format!("Unsupported SOCKS command: {}", req_header[1]));
    }

    // Parse target address
    let target_host = match req_header[3] {
        0x01 => {
            // IPv4
            let mut ip = [0u8; 4];
            client_stream
                .read_exact(&mut ip)
                .await
                .map_err(|e| format!("Failed reading IPv4: {}", e))?;
            format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
        }
        0x03 => {
            // Domain
            let mut len_buf = [0u8; 1];
            client_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(|e| format!("Failed reading domain length: {}", e))?;
            let len = len_buf[0] as usize;
            let mut domain = vec![0u8; len];
            client_stream
                .read_exact(&mut domain)
                .await
                .map_err(|e| format!("Failed reading domain name: {}", e))?;
            String::from_utf8(domain).map_err(|e| format!("Invalid domain UTF-8: {}", e))?
        }
        0x04 => {
            // IPv6
            let mut ip = [0u8; 16];
            client_stream
                .read_exact(&mut ip)
                .await
                .map_err(|e| format!("Failed reading IPv6: {}", e))?;
            // Format IPv6 cleanly
            let mut parts = vec![];
            for chunk in ip.chunks(2) {
                parts.push(format!("{:x}{:02x}", chunk[0], chunk[1]));
            }
            format!("[{}]", parts.join(":"))
        }
        _ => {
            client_stream
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await
                .ok();
            return Err(format!("Unsupported SOCKS address type: {}", req_header[3]));
        }
    };

    let mut port_buf = [0u8; 2];
    client_stream
        .read_exact(&mut port_buf)
        .await
        .map_err(|e| format!("Failed reading target port: {}", e))?;
    let target_port = u16::from_be_bytes(port_buf);

    log::debug!(
        "Client {} requesting connection to {}:{}",
        client_addr,
        target_host,
        target_port
    );

    // 3. Connect to upstream HTTP/HTTPS proxy
    let upstream_stream = TcpStream::connect((upstream_host, upstream_port))
        .await
        .map_err(|e| format!("Failed to connect to upstream proxy: {}", e))?;

    let mut upstream_boxed: BoxedStream = if upstream_scheme == "https" {
        // HTTPS connection to the proxy: wrap in rustls
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let server_name = rustls::pki_types::ServerName::try_from(upstream_host.to_string())
            .map_err(|e| format!("Invalid DNS name: {}", e))?
            .to_owned();

        let tls_stream = connector
            .connect(server_name, upstream_stream)
            .await
            .map_err(|e| format!("TLS handshake with upstream proxy failed: {}", e))?;

        Box::new(tls_stream)
    } else {
        Box::new(upstream_stream)
    };

    // 4. Send HTTP CONNECT request
    let mut connect_req = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\nProxy-Connection: Keep-Alive\r\n",
        target_host, target_port, target_host, target_port
    );

    if !upstream_username.is_empty() {
        let auth = format!("{}:{}", upstream_username, upstream_password);
        let encoded = BASE64_STANDARD.encode(auth);
        connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", encoded));
    }
    connect_req.push_str("\r\n");

    upstream_boxed
        .write_all(connect_req.as_bytes())
        .await
        .map_err(|e| format!("Failed to send CONNECT request: {}", e))?;
    upstream_boxed
        .flush()
        .await
        .map_err(|e| format!("Failed to flush CONNECT request: {}", e))?;

    // Read response until \r\n\r\n
    let mut response_headers = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        upstream_boxed
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("Error reading CONNECT response: {}", e))?;
        response_headers.push(buf[0]);
        if response_headers.ends_with(b"\r\n\r\n") {
            break;
        }
        if response_headers.len() > 8192 {
            return Err("Proxy CONNECT response headers too long".into());
        }
    }

    let response_str = String::from_utf8_lossy(&response_headers);
    if !response_str.starts_with("HTTP/1.1 200") && !response_str.starts_with("HTTP/1.0 200") {
        let status = response_str.lines().next().unwrap_or("Unknown status");
        client_stream
            .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .ok();
        return Err(format!("Proxy connection rejected: {}", status));
    }

    // 5. Send SOCKS5 success reply to client
    client_stream
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|e| format!("Failed to send SOCKS5 success reply: {}", e))?;
    client_stream
        .flush()
        .await
        .map_err(|e| format!("Failed to flush client stream: {}", e))?;

    // 6. Relay traffic bidirectionally
    let (mut client_read, mut client_write) = tokio::io::split(client_stream);
    let (mut upstream_read, mut upstream_write) = tokio::io::split(upstream_boxed);

    let client_to_upstream = tokio::io::copy(&mut client_read, &mut upstream_write);
    let upstream_to_client = tokio::io::copy(&mut upstream_read, &mut client_write);

    tokio::select! {
        res = client_to_upstream => { res.map_err(|e| format!("Relay client to upstream error: {}", e))?; }
        res = upstream_to_client => { res.map_err(|e| format!("Relay upstream to client error: {}", e))?; }
    }

    Ok(())
}
