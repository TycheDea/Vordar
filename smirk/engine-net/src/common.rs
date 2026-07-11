// Shared wire-level pieces: framing, control messages, TLS setup.

use crate::NetError;
use std::sync::Arc;

pub(crate) const TAG_CTRL: u8 = 0;
pub(crate) const TAG_APP: u8 = 1;

/// Hard cap on inbound frames (client→server). ~1 KiB is ample.
pub const MAX_FRAME_IN: usize = 1024;
/// Hard cap on outbound / client-read frames (server→client).
/// Sized for worst-case snapshot (64 entities) with headroom.
pub const MAX_FRAME_OUT: usize = 64 * 1024;

/// ALPN identifier — QUIC requires one; both sides must agree.
pub(crate) const ALPN: &[u8] = b"vordar/1";

/// Control messages handled inside engine-net (never surfaced to the game).
/// Times are microseconds on the sender's own monotonic epoch.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Ctrl {
    Hello { version: u8 },
    HelloAck,
    Ping { t_client: u64 },
    Pong { t_client: u64, t_server: u64 },
}

pub(crate) async fn write_frame(
    send: &mut quinn::SendStream,
    tag: u8,
    payload: &[u8],
) -> Result<(), quinn::WriteError> {
    let len = (payload.len() + 1) as u32;
    debug_assert!((len as usize) <= MAX_FRAME_OUT, "outbound frame exceeds MAX_FRAME_OUT");
    send.write_all(&len.to_le_bytes()).await?;
    send.write_all(&[tag]).await?;
    send.write_all(payload).await?;
    Ok(())
}

/// Server reader (client→server) uses the small inbound cap.
pub(crate) async fn read_frame_in(recv: &mut quinn::RecvStream) -> Result<(u8, Vec<u8>), NetError> {
    read_frame_cap(recv, MAX_FRAME_IN).await
}
/// Client reader (server→client) uses the large outbound cap.
pub(crate) async fn read_frame_out(recv: &mut quinn::RecvStream) -> Result<(u8, Vec<u8>), NetError> {
    read_frame_cap(recv, MAX_FRAME_OUT).await
}

async fn read_frame_cap(recv: &mut quinn::RecvStream, max: usize) -> Result<(u8, Vec<u8>), NetError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(|_| NetError::Closed)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > max {
        return Err(NetError::Handshake(format!("bad frame length {len}")));
    }
    let mut tag = [0u8; 1];
    recv.read_exact(&mut tag).await.map_err(|_| NetError::Closed)?;
    let mut buf = vec![0u8; len - 1];
    recv.read_exact(&mut buf).await.map_err(|_| NetError::Closed)?;
    Ok((tag[0], buf))
}

pub(crate) fn encode_ctrl(msg: &Ctrl) -> Vec<u8> {
    postcard::to_allocvec(msg).expect("Ctrl serialization cannot fail")
}

pub(crate) fn decode_ctrl(bytes: &[u8]) -> Option<Ctrl> {
    postcard::from_bytes(bytes).ok()
}

pub(crate) fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Server TLS config from a freshly generated self-signed certificate (dev mode).
pub(crate) fn server_crypto() -> Result<quinn::ServerConfig, NetError> {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .map_err(|e| NetError::Tls(e.to_string()))?;
    let cert_der = certified.cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()),
    );

    let mut tls = rustls::ServerConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic)))
}

/// Client TLS config that accepts any server certificate — DEV ONLY, pairs with
/// the self-signed server cert above. Encryption stays on; authentication is off.
pub(crate) fn client_crypto() -> Result<quinn::ClientConfig, NetError> {
    let mut tls = rustls::ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification(crypto_provider())))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic)))
}

#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
