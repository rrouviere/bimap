use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub fn generate_ephemeral_cert(
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>, String), String> {
    let key_pair = KeyPair::generate().map_err(|e| format!("key generation: {e}"))?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "bimap-ephemeral");
    params.distinguished_name = dn;
    params.subject_alt_names.push(rcgen::SanType::DnsName(
        "localhost".try_into().map_err(|e| format!("san: {e}"))?,
    ));

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("cert generation: {e}"))?;

    let cert_der: CertificateDer<'static> = cert.der().clone();
    let key_der: PrivateKeyDer<'static> = PrivateKeyDer::try_from(key_pair.serialize_der())
        .map_err(|e| format!("key der conversion: {e}"))?;

    let fingerprint = {
        let mut hasher = Sha256::new();
        hasher.update(cert_der.as_ref());
        format!("SHA256:{:x}", hasher.finalize())
    };

    Ok((cert_der, key_der, fingerprint))
}

pub fn make_tls_acceptor(
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
) -> Result<TlsAcceptor, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("server config: {e}"))?;
    let config = config
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| format!("cert config: {e}"))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn make_tls_connector() -> Result<TlsConnector, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("client config: {e}"))?;
    let config = config
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub async fn server_tls_accept(
    acceptor: &TlsAcceptor,
    listener: &TcpListener,
) -> Result<
    (
        tokio_rustls::server::TlsStream<TcpStream>,
        std::net::SocketAddr,
    ),
    String,
> {
    let (stream, addr) = listener
        .accept()
        .await
        .map_err(|e| format!("accept: {e}"))?;
    let tls_stream = acceptor
        .accept(stream)
        .await
        .map_err(|e| format!("tls accept: {e}"))?;
    Ok((tls_stream, addr))
}

pub async fn client_tls_connect(
    connector: &TlsConnector,
    target: SocketAddr,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let stream = TcpStream::connect(target)
        .await
        .map_err(|e| format!("connect {target}: {e}"))?;
    let server_name =
        ServerName::try_from("localhost").map_err(|_| "bad server name".to_string())?;
    let tls_stream = connector
        .connect(server_name, stream)
        .await
        .map_err(|e| format!("tls connect: {e}"))?;
    Ok(tls_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_generation_produces_fingerprint() {
        let (cert, _key, fingerprint) = generate_ephemeral_cert().unwrap();
        assert!(!cert.is_empty());
        assert!(!fingerprint.is_empty());
        assert!(fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn fingerprint_is_unique() {
        let (_, _, fp1) = generate_ephemeral_cert().unwrap();
        let (_, _, fp2) = generate_ephemeral_cert().unwrap();
        assert_ne!(fp1, fp2, "ephemeral certs should have unique fingerprints");
    }
}
