use meshway_transport_trojan::{
    ClientConfig, SecretPassword, ServerConfig, Target,
    tls::{
        ClientTls, ServerTls,
        rustls::{RootCertStore, pki_types::PrivatePkcs8KeyDer},
    },
};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose,
};

pub const SERVER_NAME: &str = "trojan.meshway.test";

pub struct Certificates {
    pub ca: Certificate,
    pub leaf: Certificate,
    pub key: KeyPair,
}

impl Certificates {
    pub fn new(expired: bool) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap();
        let issuer = Issuer::new(params, key);
        let mut params = CertificateParams::new(vec![SERVER_NAME.to_owned()]).unwrap();
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        if expired {
            params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let leaf = params.signed_by(&key, &issuer).unwrap();
        Self { ca, leaf, key }
    }

    pub fn roots(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(self.ca.der().clone()).unwrap();
        roots
    }

    pub fn client(&self) -> ClientTls {
        ClientTls::new(SERVER_NAME, self.roots()).unwrap()
    }

    pub fn server(&self) -> ServerTls {
        ServerTls::new(
            vec![self.leaf.der().clone()],
            PrivatePkcs8KeyDer::from(self.key.serialize_der()).into(),
        )
        .unwrap()
    }
}

pub fn configs() -> (ClientConfig, ServerConfig) {
    let password = SecretPassword::new(b"ephemeral unit test credential").unwrap();
    let target = Target::domain("meshway.endpoint.test", 4242).unwrap();
    (
        ClientConfig {
            password: password.clone(),
            target: target.clone(),
        },
        ServerConfig {
            password,
            expected_target: target,
        },
    )
}
