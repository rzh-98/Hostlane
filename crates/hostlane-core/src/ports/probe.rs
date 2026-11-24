//! What a port answers: an HTTP request over a `direct-tcpip` channel, so the request
//! comes from the server itself and nothing has to be installed there. HTTPS is tried
//! with a verifier that accepts any certificate, because the point is to report the
//! certificate, not to trust it.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use hostlane_protocol::{HttpProbe, TlsInfo};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::cert::parse_certificate;
use crate::ssh::SshHandle;

/// Each attempt (plain or TLS), from opening the channel to the end of the response.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE: usize = 64 * 1024;
const MAX_TITLE: usize = 200;
/// Ports where HTTPS is tried before plain HTTP.
const TLS_PORTS: &[u16] = &[443, 4443, 6443, 8443, 9443, 10250];

/// Result of probing one port.
#[derive(Debug, Default)]
pub struct Outcome {
    pub protocol: Option<String>,
    pub http: Option<HttpProbe>,
    pub error: Option<String>,
    /// The SSH server refuses `direct-tcpip`; probing other ports is pointless.
    pub forwarding_denied: bool,
}

#[derive(Debug)]
enum OpenError {
    Refused,
    Prohibited,
    Other(String),
}

impl OpenError {
    fn message(&self) -> String {
        match self {
            Self::Refused => "connection refused".into(),
            Self::Prohibited => {
                "the SSH server does not allow TCP forwarding (AllowTcpForwarding)".into()
            }
            Self::Other(message) => message.clone(),
        }
    }
}

async fn open(
    handle: &SshHandle,
    address: &str,
    port: u16,
) -> Result<russh::ChannelStream<russh::client::Msg>, OpenError> {
    use russh::ChannelOpenFailure as Failure;
    match handle
        .channel_open_direct_tcpip(address, port.into(), "127.0.0.1", 0)
        .await
    {
        Ok(channel) => Ok(channel.into_stream()),
        Err(russh::Error::ChannelOpenFailure(Failure::ConnectFailed)) => Err(OpenError::Refused),
        Err(russh::Error::ChannelOpenFailure(Failure::AdministrativelyProhibited)) => {
            Err(OpenError::Prohibited)
        }
        Err(err) => Err(OpenError::Other(format!("cannot open a channel: {err}"))),
    }
}

/// What came back from one request.
struct Exchange {
    data: Vec<u8>,
    first_byte: Option<Duration>,
    timed_out: bool,
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: &[u8],
    deadline: tokio::time::Instant,
) -> Result<Exchange, String> {
    let sent = Instant::now();
    tokio::time::timeout_at(deadline, async {
        stream.write_all(request).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| "timed out".to_string())?
    .map_err(|err| err.to_string())?;
    let mut out = Exchange {
        data: Vec::new(),
        first_byte: None,
        timed_out: false,
    };
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let read = match tokio::time::timeout_at(deadline, stream.read(&mut buf)).await {
            Err(_) => {
                out.timed_out = true;
                break;
            }
            Ok(Err(err)) if out.data.is_empty() => return Err(err.to_string()),
            Ok(Err(_)) | Ok(Ok(0)) => break,
            Ok(Ok(read)) => read,
        };
        out.first_byte.get_or_insert_with(|| sent.elapsed());
        out.data.extend_from_slice(&buf[..read]);
        if out.data.len() >= MAX_RESPONSE || complete(&out.data) {
            break;
        }
    }
    out.data.truncate(MAX_RESPONSE);
    Ok(out)
}

fn request(host_header: &str) -> Vec<u8> {
    format!(
        "GET / HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: hostlane-probe\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

fn host_header(address: &str, port: u16, server_name: Option<&str>) -> String {
    match server_name {
        Some(name) => name.to_string(),
        None if address.contains(':') => format!("[{address}]:{port}"),
        None => format!("{address}:{port}"),
    }
}

enum Attempt {
    Http(HttpProbe),
    /// An HTTP error that says the port speaks HTTPS.
    WrongScheme(HttpProbe),
    Banner(&'static str),
    NotHttp,
    /// Accepted the connection and said nothing until the timeout. TLS servers
    /// answer garbage at once, so this is not worth a TLS attempt.
    Silent,
}

fn classify(scheme: &str, exchange: &Exchange, tls: Option<TlsInfo>) -> Attempt {
    let data = &exchange.data;
    if let Some(head) = parse_head(data) {
        let body = &data[head.body_start..];
        let probe = HttpProbe {
            scheme: scheme.into(),
            status: head.status,
            title: if html(&head) { title(body) } else { None },
            server: head.header("server"),
            content_type: head.header("content-type"),
            location: head.header("location"),
            latency_ms: exchange.first_byte.unwrap_or_default().as_millis() as u64,
            tls,
        };
        if scheme == "http" && head.status == 400 && wants_tls(body) {
            return Attempt::WrongScheme(probe);
        }
        return Attempt::Http(probe);
    }
    match banner_protocol(data) {
        Some(name) if scheme == "http" => Attempt::Banner(name),
        _ => Attempt::NotHttp,
    }
}

async fn plain(
    handle: &SshHandle,
    address: &str,
    port: u16,
    host: &str,
) -> Result<Attempt, OpenError> {
    let deadline = tokio::time::Instant::now() + ATTEMPT_TIMEOUT;
    let mut stream = tokio::time::timeout_at(deadline, open(handle, address, port))
        .await
        .map_err(|_| OpenError::Other("timed out".into()))??;
    let exchange = exchange(&mut stream, &request(host), deadline)
        .await
        .map_err(OpenError::Other)?;
    let _ = stream.shutdown().await;
    if exchange.data.starts_with(&[0x15, 0x03]) {
        return Ok(Attempt::NotHttp);
    }
    if exchange.data.is_empty() && exchange.timed_out {
        return Ok(Attempt::Silent);
    }
    Ok(classify("http", &exchange, None))
}

fn tls_version(version: rustls::ProtocolVersion) -> Option<String> {
    match version {
        rustls::ProtocolVersion::TLSv1_3 => Some("TLSv1.3".into()),
        rustls::ProtocolVersion::TLSv1_2 => Some("TLSv1.2".into()),
        _ => None,
    }
}

async fn tls(
    handle: &SshHandle,
    address: &str,
    port: u16,
    server_name: Option<&str>,
    host: &str,
    now_ms: u64,
) -> Result<Attempt, OpenError> {
    let deadline = tokio::time::Instant::now() + ATTEMPT_TIMEOUT;
    let stream = tokio::time::timeout_at(deadline, open(handle, address, port))
        .await
        .map_err(|_| OpenError::Other("timed out".into()))??;
    let name = server_name
        .and_then(|name| ServerName::try_from(name.to_string()).ok())
        .or_else(|| ServerName::try_from(address.to_string()).ok())
        .ok_or_else(|| OpenError::Other(format!("invalid server name `{address}`")))?;
    let connector = tokio_rustls::TlsConnector::from(tls_config());
    let mut stream = tokio::time::timeout_at(deadline, connector.connect(name, stream))
        .await
        .map_err(|_| OpenError::Other("TLS handshake timed out".into()))?
        .map_err(|err| OpenError::Other(format!("TLS handshake failed: {err}")))?;
    let info = {
        let (_, session) = stream.get_ref();
        let mut info = session
            .peer_certificates()
            .and_then(|certs| certs.first())
            .and_then(|cert| parse_certificate(cert.as_ref(), now_ms))
            .unwrap_or_default();
        info.version = session.protocol_version().and_then(tls_version);
        info
    };
    let exchange = exchange(&mut stream, &request(host), deadline)
        .await
        .map_err(OpenError::Other)?;
    let _ = stream.shutdown().await;
    match classify("https", &exchange, Some(info)) {
        Attempt::NotHttp => Err(OpenError::Other(
            "TLS works but no HTTP response came back".into(),
        )),
        attempt => Ok(attempt),
    }
}

/// Probes `address:port` on the server. `server_name` sets the `Host` header and SNI.
pub async fn probe(
    handle: &SshHandle,
    address: &str,
    port: u16,
    server_name: Option<&str>,
    now_ms: u64,
) -> Outcome {
    let host = host_header(address, port, server_name);
    let tls_first = TLS_PORTS.contains(&port);
    let mut fallback: Option<HttpProbe> = None;
    let mut last_error: Option<String> = None;
    for use_tls in [tls_first, !tls_first] {
        let attempt = if use_tls {
            tls(handle, address, port, server_name, &host, now_ms).await
        } else {
            plain(handle, address, port, &host).await
        };
        match attempt {
            Ok(Attempt::Http(http)) => {
                return Outcome {
                    protocol: Some(http.scheme.clone()),
                    http: Some(http),
                    ..Outcome::default()
                };
            }
            Ok(Attempt::WrongScheme(http)) => fallback = Some(http),
            Ok(Attempt::Banner(name)) => {
                return Outcome {
                    protocol: Some(name.into()),
                    ..Outcome::default()
                };
            }
            Ok(Attempt::NotHttp) => {}
            Ok(Attempt::Silent) => break,
            Err(err @ (OpenError::Refused | OpenError::Prohibited)) => {
                return Outcome {
                    error: Some(err.message()),
                    forwarding_denied: matches!(err, OpenError::Prohibited),
                    ..Outcome::default()
                };
            }
            Err(err) => last_error = Some(err.message()),
        }
    }
    match fallback {
        Some(http) => Outcome {
            protocol: Some("http".into()),
            http: Some(http),
            ..Outcome::default()
        },
        None => Outcome {
            error: Some(match last_error {
                Some(err) if tls_first => err,
                _ => "no HTTP response".into(),
            }),
            ..Outcome::default()
        },
    }
}

#[derive(Debug)]
struct AcceptAnyCertificate(Vec<SignatureScheme>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.clone()
    }
}

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let schemes = provider
                .signature_verification_algorithms
                .supported_schemes();
            let mut config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports the default TLS versions")
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(schemes)))
                .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}

/// Status line and headers of an HTTP response.
#[derive(Debug)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body_start: usize,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// Parses the head once it is complete; `None` for anything that is not HTTP.
pub fn parse_head(data: &[u8]) -> Option<Head> {
    if !data.starts_with(b"HTTP/") {
        return None;
    }
    let (end, body_start) = match find(data, b"\r\n\r\n") {
        Some(end) => (end, end + 4),
        None => {
            let end = find(data, b"\n\n")?;
            (end, end + 2)
        }
    };
    let text = String::from_utf8_lossy(&data[..end]);
    let mut lines = text.lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Some(Head {
        status,
        headers,
        body_start,
    })
}

fn html(head: &Head) -> bool {
    head.header("content-type")
        .is_none_or(|t| t.to_ascii_lowercase().contains("html"))
}

/// Enough of the response has arrived to report on it.
fn complete(data: &[u8]) -> bool {
    let Some(head) = parse_head(data) else {
        return !data.is_empty() && !data.starts_with(b"HTTP/") && !b"HTTP/".starts_with(data);
    };
    let body = &data[head.body_start..];
    if (100..200).contains(&head.status) || matches!(head.status, 204 | 304) {
        return true;
    }
    if let Some(length) = head
        .header("content-length")
        .and_then(|l| l.parse::<usize>().ok())
        && body.len() >= length
    {
        return true;
    }
    !html(&head)
        || find_ignore_case(body, b"</title").is_some()
        || find_ignore_case(body, b"</head").is_some()
}

/// nginx and Go answer plain HTTP on a TLS port with a 400 that says so.
fn wants_tls(body: &[u8]) -> bool {
    find_ignore_case(body, b"sent to HTTPS port").is_some()
        || find_ignore_case(body, b"HTTP request to an HTTPS server").is_some()
}

/// Protocols that speak first, recognised by their greeting.
pub fn banner_protocol(data: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(&data[..data.len().min(256)]);
    if text.starts_with("SSH-") {
        return Some("ssh");
    }
    if text.starts_with("220") {
        let ftp = text.to_ascii_lowercase().contains("ftp");
        return Some(if ftp { "ftp" } else { "smtp" });
    }
    if text.starts_with("* OK") {
        return Some("imap");
    }
    if text.starts_with("+OK") {
        return Some("pop3");
    }
    if ["-ERR", "-DENIED", "-NOAUTH"]
        .iter()
        .any(|p| text.starts_with(p))
    {
        return Some("redis");
    }
    if data.len() > 5 && data[3] == 0 && data[4] == 10 {
        return Some("mysql");
    }
    None
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest
            .char_indices()
            .take(12)
            .find(|(_, c)| *c == ';')
            .and_then(|(end, _)| {
                let entity = &rest[1..end];
                let c = match entity {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    "nbsp" => ' ',
                    _ => {
                        let code = match entity.strip_prefix("#x").or(entity.strip_prefix("#X")) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                            None => entity.strip_prefix('#')?.parse().ok()?,
                        };
                        char::from_u32(code)?
                    }
                };
                Some((c, end))
            });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of the first `<title>`, whitespace collapsed and capped.
pub fn title(body: &[u8]) -> Option<String> {
    let start = find_ignore_case(body, b"<title")?;
    let open_end = start + body[start..].iter().position(|b| *b == b'>')? + 1;
    let close = open_end + find_ignore_case(&body[open_end..], b"</title")?;
    let raw = String::from_utf8_lossy(&body[open_end..close]);
    let text = decode_entities(&raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(MAX_TITLE) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_heads_and_titles() {
        let response = b"HTTP/1.1 200 OK\r\nServer: nginx/1.27\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><TITLE>\n  Grafana &amp; Friends &#x2014; Home\n</TITLE></head>";
        let head = parse_head(response).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("server").as_deref(), Some("nginx/1.27"));
        assert!(complete(response));
        assert_eq!(
            title(&response[head.body_start..]).as_deref(),
            Some("Grafana & Friends \u{2014} Home")
        );

        let partial = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><head>";
        assert!(!complete(partial));
        assert!(!complete(b"HTT"));
        let json = b"HTTP/1.0 404 Not Found\r\nContent-Type: application/json\r\n\r\n";
        assert!(complete(json));
        let sized = b"HTTP/1.1 301 Moved\r\nLocation: /login\r\nContent-Length: 0\r\n\r\n";
        assert!(complete(sized));
        assert_eq!(
            parse_head(sized).unwrap().header("location").as_deref(),
            Some("/login")
        );
        assert!(parse_head(b"SSH-2.0-OpenSSH_9.6\r\n").is_none());
        assert_eq!(title(b"<title> </title>"), None);
        assert_eq!(
            title(b"<title>a &bogus; &#60;b&#62;</title>").as_deref(),
            Some("a &bogus; <b>")
        );
        let long = format!("<title>{}</title>", "x".repeat(300));
        let capped = title(long.as_bytes()).unwrap();
        assert_eq!(capped.chars().count(), MAX_TITLE + 1);
    }

    #[test]
    fn classification() {
        let exchange = |data: &[u8]| Exchange {
            data: data.to_vec(),
            first_byte: Some(Duration::from_millis(5)),
            timed_out: false,
        };
        let nginx = exchange(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><head><title>400 The plain HTTP request was sent to HTTPS port</title></head>");
        assert!(matches!(
            classify("http", &nginx, None),
            Attempt::WrongScheme(_)
        ));
        match classify("http", &exchange(b"HTTP/1.1 200 OK\r\n\r\n"), None) {
            Attempt::Http(probe) => assert_eq!((probe.status, probe.latency_ms), (200, 5)),
            _ => panic!("expected HTTP"),
        }
        assert!(matches!(
            classify("http", &exchange(b"SSH-2.0-OpenSSH_9.6\r\n"), None),
            Attempt::Banner("ssh")
        ));
        assert!(matches!(
            classify("https", &exchange(b"SSH-2.0-OpenSSH_9.6\r\n"), None),
            Attempt::NotHttp
        ));
        assert_eq!(
            banner_protocol(b"220 mail.example.com ESMTP Postfix"),
            Some("smtp")
        );
        assert_eq!(banner_protocol(b"220 (vsFTPd 3.0.5)"), Some("ftp"));
        assert_eq!(
            banner_protocol(b"\x4a\x00\x00\x00\x0a8.0.36\x00"),
            Some("mysql")
        );
        assert_eq!(banner_protocol(b"garbage"), None);
        assert_eq!(host_header("::1", 80, None), "[::1]:80");
        assert_eq!(
            host_header("127.0.0.1", 80, Some("example.com")),
            "example.com"
        );
    }
}
