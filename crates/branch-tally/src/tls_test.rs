//! Test-only loopback TLS server and a matching client agent (#98). The shipped binary has no
//! hook to trust another CA: only tests build an agent that trusts the committed fixture CA.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

const CA: &[u8] = include_bytes!("../tests/fixtures/tls/ca.der");
const LEAF: &[u8] = include_bytes!("../tests/fixtures/tls/leaf.der");
const LEAF_KEY: &[u8] = include_bytes!("../tests/fixtures/tls/leaf.key.der");

/// A one-shot HTTPS server on loopback. The host in `url` is `localhost`, as the leaf requires.
pub(crate) struct TlsServer {
    pub(crate) url: String,
    handle: JoinHandle<bool>,
}

impl TlsServer {
    /// Waits for the server thread; `true` when a client connected and was answered.
    pub(crate) fn finish(self) -> bool {
        self.handle.join().expect("server thread")
    }
}

/// Serves one request with `status` (e.g. `"200 OK"`), `extra_headers` (each ending in CRLF) and
/// `body`.
pub(crate) fn serve_tls_once(status: &str, extra_headers: &str, body: &str) -> TlsServer {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(LEAF)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(LEAF_KEY)),
            )
            .expect("fixture cert and key"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let reply = format!(
        "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut tcp = loop {
            match listener.accept() {
                Ok((tcp, _)) => break tcp,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return false,
            }
        };
        tcp.set_nonblocking(false).expect("blocking");
        let Ok(mut conn) = rustls::ServerConnection::new(config) else {
            return false;
        };
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        let mut buf = [0u8; 4096];
        if tls.read(&mut buf).is_err() {
            return false;
        }
        let _ = tls.write_all(reply.as_bytes());
        tls.conn.send_close_notify();
        let _ = tls.flush();
        true
    });
    TlsServer {
        url: format!("https://localhost:{port}/feed.json"),
        handle,
    }
}

/// An https-only agent that trusts the fixture CA and nothing else.
pub(crate) fn fixture_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .https_only(true)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::new_with_certs(&[
                    Certificate::from_der(CA).to_owned()
                ]))
                .build(),
        )
        .build()
        .into()
}
