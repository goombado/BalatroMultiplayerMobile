//! Network plumbing: TLS config, CM server discovery and the WebSocket transport handed to
//! steam-vent.
//!
//! The bridge brings its own transport instead of `ServerList::discover` + steam-vent's default
//! WebSocket for three reasons:
//! - reqwest's platform verifier needs JNI setup on Android.
//! - The default transport enables rustls key logging (SSLKEYLOGFILE).
//! - We need to know when the socket closes, to reconnect.

use bytes::BytesMut;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use rustls::{ClientConfig, RootCertStore};
use std::sync::{Arc, OnceLock};
use steam_vent::NetworkError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};

const DIRECTORY_HOST: &str = "api.steampowered.com";
const CM_LIST_PATH: &str = "/ISteamDirectory/GetCMListForConnect/v1/?cellid=0&format=json";

pub fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("rustls protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// Minimal HTTPS GET. HTTP/1.0 keeps the response un-chunked; only used for the CM directory.
pub async fn https_get(host: &str, path: &str) -> Result<Vec<u8>, String> {
    let tcp = tokio::net::TcpStream::connect((host, 443)).await.map_err(|e| format!("connect {host}: {e}"))?;
    let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|e| e.to_string())?;
    let mut tls = tokio_rustls::TlsConnector::from(tls_config()).connect(name, tcp).await.map_err(|e| format!("tls {host}: {e}"))?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: bmp-steam-bridge\r\nAccept: application/json\r\n\r\n");
    tls.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut resp = Vec::new();
    // Servers often close without close_notify; whatever arrived is still the full HTTP/1.0 body.
    if let Err(e) = tls.read_to_end(&mut resp).await {
        if resp.is_empty() {
            return Err(e.to_string());
        }
    }
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n").ok_or("malformed HTTP response")?;
    let head = String::from_utf8_lossy(&resp[..split]);
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(format!("HTTP {status} from {host}"));
    }
    Ok(resp[split + 4..].to_vec())
}

/// Blocking `https_get` for the desktop harness, on its own throwaway runtime.
#[doc(hidden)]
pub fn https_get_blocking(host: &str, path: &str) -> Result<Vec<u8>, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?.block_on(https_get(host, path))
}

/// WebSocket CM endpoints ("host:port"), in Valve's preference order.
pub fn parse_cm_list(json: &[u8]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Server {
        endpoint: String,
        #[serde(rename = "type")]
        kind: String,
    }
    #[derive(serde::Deserialize)]
    struct Inner {
        #[serde(default)]
        serverlist: Vec<Server>,
    }
    #[derive(serde::Deserialize)]
    struct Outer {
        response: Inner,
    }
    serde_json::from_slice::<Outer>(json)
        .map(|o| o.response.serverlist.into_iter().filter(|s| s.kind == "websockets").map(|s| s.endpoint).collect())
        .unwrap_or_default()
}

pub async fn cm_servers() -> Result<Vec<String>, String> {
    let body = https_get(DIRECTORY_HOST, CM_LIST_PATH).await?;
    let list = parse_cm_list(&body);
    if list.is_empty() {
        return Err("Steam returned no WebSocket CM servers".into());
    }
    Ok(list)
}

pub type CmSink = std::pin::Pin<Box<dyn Sink<BytesMut, Error = NetworkError> + Send>>;
pub type CmStream = std::pin::Pin<Box<dyn Stream<Item = Result<BytesMut, NetworkError>> + Send>>;

/// Connects to `wss://<endpoint>/cmsocket/`. The watch flips to `true` once the socket has closed or failed.
pub async fn connect_cm(endpoint: &str) -> Result<(CmSink, CmStream, watch::Receiver<bool>), String> {
    let url = format!("wss://{endpoint}/cmsocket/");
    let (ws, _) = connect_async_tls_with_config(url.as_str(), None, false, Some(Connector::Rustls(tls_config())))
        .await
        .map_err(|e| format!("websocket {endpoint}: {e}"))?;
    let (write, read) = ws.split();
    let (closed_tx, closed_rx) = watch::channel(false);

    let sink = write.sink_map_err(NetworkError::from).with(|msg: BytesMut| futures_util::future::ready(Ok::<_, NetworkError>(Message::binary(msg.freeze()))));

    // Only binary frames carry Steam messages; tungstenite answers pings itself.
    let stream = futures_util::stream::unfold((read, Some(closed_tx)), |(mut read, mut closed)| async move {
        loop {
            match read.next().await {
                Some(Ok(Message::Binary(b))) => return Some((Ok(BytesMut::from(b.as_ref())), (read, closed))),
                Some(Ok(Message::Close(_))) | None => {
                    if let Some(tx) = closed.take() {
                        let _ = tx.send(true);
                    }
                    return None;
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => {
                    if let Some(tx) = closed.take() {
                        let _ = tx.send(true);
                    }
                    return Some((Err(NetworkError::from(e)), (read, None)));
                }
            }
        }
    });

    Ok((Box::pin(sink), Box::pin(stream), closed_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cm_list_keeps_websockets_in_order() {
        let json = br#"{"response":{"serverlist":[
            {"endpoint":"a.example:27017","type":"netfilter"},
            {"endpoint":"ws1.example:443","type":"websockets"},
            {"endpoint":"ws2.example:443","type":"websockets"}],"success":true}}"#;
        assert_eq!(parse_cm_list(json), vec!["ws1.example:443", "ws2.example:443"]);
        assert!(parse_cm_list(b"{}").is_empty());
    }
}
