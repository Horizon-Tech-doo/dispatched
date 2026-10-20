//! Opening the AMQP connection: TLS on 5671, plain on the emulator, or
//! WebSockets on 443 (optionally through an HTTP CONNECT proxy).

use super::management::SETUP_TIMEOUT;
use crate::namespace::{Namespace, Transport};
use crate::{Error, Result};
use base64::Engine;
use fe2o3_amqp::Connection;
use fe2o3_amqp::connection::ConnectionHandle;
use fe2o3_amqp::sasl_profile::SaslProfile;
use secrecy::ExposeSecret;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const PROXY_TIMEOUT: Duration = Duration::from_secs(15);

/// Ask an HTTP proxy to open a tunnel to `host:port`.
pub(crate) async fn proxy_connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    host: &str,
    port: u16,
    auth: Option<(&str, &str)>,
    timeout: Duration,
) -> Result<()> {
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some((user, pass)) = auth {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    let exchange = async {
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| Error::Network(format!("proxy write: {e}")))?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let n = stream
                .read(&mut byte)
                .await
                .map_err(|e| Error::Network(format!("proxy read: {e}")))?;
            if n == 0 {
                return Err(Error::Network(
                    "the proxy closed the connection during CONNECT".into(),
                ));
            }
            head.push(byte[0]);
            if head.len() > 8192 {
                return Err(Error::Network(
                    "the proxy sent an oversized response to CONNECT".into(),
                ));
            }
        }
        let text = String::from_utf8_lossy(&head);
        let status_line = text.lines().next().unwrap_or_default();
        let mut parts = status_line.split_whitespace();
        let (Some(version), Some(code)) = (parts.next(), parts.next()) else {
            return Err(Error::Network(
                "the proxy sent an invalid response to CONNECT".into(),
            ));
        };
        if !version.starts_with("HTTP/1.") {
            return Err(Error::Network(
                "the proxy sent an invalid response to CONNECT".into(),
            ));
        }
        if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Network(
                "the proxy sent an invalid response to CONNECT".into(),
            ));
        }
        match code {
            "200" => Ok(()),
            "407" => Err(Error::Network(
                "the proxy requires authentication (407); set HTTPS_PROXY=http://user:pass@host:port and remove the profile's proxy (profiles store no credentials, and a profile proxy takes precedence over HTTPS_PROXY)".into(),
            )),
            other => Err(Error::Network(format!("the proxy refused the tunnel to {host}:{port} (HTTP {other})"))),
        }
    };
    tokio::time::timeout(timeout, exchange).await.map_err(|_| {
        Error::Timeout(format!(
            "the proxy did not answer CONNECT within {}s",
            timeout.as_secs()
        ))
    })?
}

/// Open the AMQP connection for `ns` using its transport.
pub(crate) async fn open_connection(
    ns: &Namespace,
    container_id: String,
) -> Result<ConnectionHandle<()>> {
    crate::tls::ensure_crypto_provider();
    let builder = Connection::builder()
        .container_id(container_id)
        .hostname(Some(ns.host()))
        .sasl_profile(SaslProfile::Anonymous)
        .max_frame_size(65_536u32)
        .idle_time_out(60_000u32);
    match ns.transport() {
        Transport::Amqp => {
            let url = ns.amqp_url();
            let builder = if ns.is_emulator() {
                builder
            } else {
                builder.alt_tls_establishment(true)
            };
            tokio::time::timeout(SETUP_TIMEOUT, builder.open(url.as_str()))
                .await
                .map_err(|_| Error::Timeout(format!("connecting to {url} timed out")))?
                .map_err(|e| Error::Network(format!("cannot connect to {url}: {e}")))
        }
        Transport::WebSockets => {
            let ws_url = format!("wss://{}/$servicebus/websocket/", ns.host());
            let connect = async {
                let ws = match ns.proxy() {
                    None => fe2o3_amqp_ws::WebSocketStream::connect(ws_url.as_str())
                        .await
                        .map_err(|e| Error::Network(format!("cannot open {ws_url}: {e}")))?,
                    Some(proxy) => {
                        let mut tcp = TcpStream::connect((proxy.host(), proxy.port()))
                            .await
                            .map_err(|e| {
                                Error::Network(format!(
                                    "cannot reach proxy {}:{}: {e}",
                                    proxy.host(),
                                    proxy.port()
                                ))
                            })?;
                        let auth = proxy.basic_auth();
                        let auth_ref = auth.as_ref().map(|(u, p)| (u.as_str(), p.expose_secret()));
                        proxy_connect(&mut tcp, ns.host(), 443, auth_ref, PROXY_TIMEOUT).await?;
                        fe2o3_amqp_ws::WebSocketStream::connect_tls_with_stream(
                            ws_url.as_str(),
                            tcp,
                        )
                        .await
                        .map_err(|e| {
                            Error::Network(format!("cannot open {ws_url} through the proxy: {e}"))
                        })?
                    }
                };
                builder.open_with_stream(ws).await.map_err(|e| {
                    Error::Network(format!("AMQP over WebSockets to {}: {e}", ns.host()))
                })
            };
            tokio::time::timeout(SETUP_TIMEOUT, connect)
                .await
                .map_err(|_| Error::Timeout(format!("connecting to {ws_url} timed out")))?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn proxy_replying(reply: &'static [u8]) -> (TcpStream, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = s.read(&mut buf).await.unwrap();
            if !reply.is_empty() {
                s.write_all(reply).await.unwrap();
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            buf.truncate(n);
            buf
        });
        (TcpStream::connect(addr).await.unwrap(), server)
    }

    #[tokio::test]
    async fn proxy_200_opens_tunnel_and_sends_auth() {
        let (mut s, server) = proxy_replying(b"HTTP/1.1 200 Connection established\r\n\r\n").await;
        proxy_connect(
            &mut s,
            "ns.servicebus.windows.net",
            443,
            Some(("u", "p")),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let req = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(req.starts_with("CONNECT ns.servicebus.windows.net:443 HTTP/1.1\r\n"));
        assert!(req.contains("Proxy-Authorization: Basic dTpw\r\n"));
    }

    #[tokio::test]
    async fn proxy_407_is_named() {
        let (mut s, _server) =
            proxy_replying(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await;
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(matches!(&e, Error::Network(m) if m.contains("407")), "{e}");
    }

    #[tokio::test]
    async fn proxy_407_points_at_https_proxy_and_never_echoes_credentials() {
        let (mut s, _server) =
            proxy_replying(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await;
        let e = proxy_connect(
            &mut s,
            "h",
            443,
            Some(("alice", "s3cret")),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("HTTPS_PROXY=http://user:pass@host:port"), "{e}");
        assert!(e.contains("remove the profile's proxy"), "{e}");
        assert!(!e.contains("alice") && !e.contains("s3cret"), "{e}");
    }

    #[tokio::test]
    async fn proxy_garbage_is_network_error() {
        let (mut s, _server) = proxy_replying(b"SSH-2.0-OpenSSH\r\n\r\n").await;
        assert!(matches!(
            proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2)).await,
            Err(Error::Network(_))
        ));
    }

    #[tokio::test]
    async fn proxy_timeout() {
        let (mut s, _server) = proxy_replying(b"").await;
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Timeout(_)));
    }

    #[tokio::test]
    async fn proxy_connect_leaves_bytes_after_the_headers_unread() {
        let (mut s, server) = proxy_replying(b"HTTP/1.1 200 OK\r\n\r\nEXTRA").await;
        proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2))
            .await
            .unwrap();
        let mut extra = [0u8; 5];
        s.read_exact(&mut extra).await.unwrap();
        assert_eq!(&extra, b"EXTRA");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn proxy_request_names_the_target_in_host_header() {
        let (mut s, server) = proxy_replying(b"HTTP/1.1 200 OK\r\n\r\n").await;
        proxy_connect(
            &mut s,
            "ns.servicebus.windows.net",
            443,
            None,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let req = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(
            req.contains("\r\nHost: ns.servicebus.windows.net:443\r\n"),
            "{req}"
        );
        assert!(!req.contains("Proxy-Authorization"), "{req}");
    }

    #[tokio::test]
    async fn proxy_refusals_are_named_without_leaking_credentials() {
        for reply in [
            &b"HTTP/1.1 403 Forbidden\r\n\r\n"[..],
            &b"HTTP/1.1 502 Bad Gateway\r\n\r\n"[..],
        ] {
            let (mut s, _server) = proxy_replying(reply).await;
            let e = proxy_connect(
                &mut s,
                "h",
                443,
                Some(("alice", "hunter2")),
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
            let text = e.to_string();
            assert!(matches!(e, Error::Network(_)), "{text}");
            assert!(
                text.contains("HTTP 403") || text.contains("HTTP 502"),
                "{text}"
            );
            assert!(
                !text.contains("hunter2") && !text.contains("alice"),
                "{text}"
            );
            assert!(!text.contains("YWxpY2U6aHVudGVyMg"), "{text}");
        }
    }

    #[tokio::test]
    async fn proxy_with_a_non_numeric_status_is_invalid_and_not_echoed() {
        let (mut s, _server) =
            proxy_replying(b"HTTP/1.1 <script>alert(1)</script> x\r\n\r\n").await;
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2))
            .await
            .unwrap_err();
        let text = e.to_string();
        assert!(text.contains("invalid response"), "{text}");
        assert!(!text.contains("script"), "{text}");
    }

    #[tokio::test]
    async fn proxy_closing_before_replying_is_an_error_not_a_hang() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
            // dropped: closes without replying
        });
        let mut s = TcpStream::connect(addr).await.unwrap();
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(
            matches!(&e, Error::Network(m) if m.contains("closed")),
            "{e}"
        );
    }
}
