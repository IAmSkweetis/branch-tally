//! Proves the committed loopback TLS fixtures work: a ureq agent that trusts `ca.der` can GET from
//! a rustls server using `leaf.der`, and the default (bundled roots) agent refuses it (#98).

#![expect(clippy::expect_used, reason = "test helpers fail loudly")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

const CA: &[u8] = include_bytes!("fixtures/tls/ca.der");
const LEAF: &[u8] = include_bytes!("fixtures/tls/leaf.der");
const LEAF_KEY: &[u8] = include_bytes!("fixtures/tls/leaf.key.der");

/// Serves `body` once over TLS on loopback and returns the `https://` URL.
fn serve_tls_once(body: &'static str) -> String {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(LEAF)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(LEAF_KEY)),
        )
        .expect("fixture cert and key");
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let Ok((mut tcp, _)) = listener.accept() else {
            return;
        };
        let Ok(mut conn) = rustls::ServerConnection::new(config) else {
            return;
        };
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        let mut buf = [0u8; 4096];
        if tls.read(&mut buf).is_err() {
            return;
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tls.write_all(head.as_bytes());
        tls.conn.send_close_notify();
        let _ = tls.flush();
    });
    format!("https://localhost:{port}/feed.json")
}

#[test]
fn agent_trusting_the_fixture_ca_gets_from_the_loopback_server() {
    let url = serve_tls_once("{}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::new_with_certs(&[
                    Certificate::from_der(CA).to_owned()
                ]))
                .build(),
        )
        .build()
        .into();
    let body = agent
        .get(&url)
        .call()
        .expect("GET")
        .body_mut()
        .read_to_string()
        .expect("body");
    assert_eq!(body, "{}");
}

#[test]
fn default_agent_rejects_the_fixture_ca() {
    let url = serve_tls_once("{}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .build()
        .into();
    assert!(agent.get(&url).call().is_err());
}
