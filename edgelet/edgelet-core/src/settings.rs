// Copyright (c) Microsoft. All rights reserved.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use regex::Regex;
use url::Url;
use url_serde;

use crate::crypto::MemoryKey;
use crate::error::{Error, ErrorKind};
use crate::module::ModuleSpec;

const DEVICEID_KEY: &str = "DeviceId";
const HOSTNAME_KEY: &str = "HostName";
const SHAREDACCESSKEY_KEY: &str = "SharedAccessKey";

const DEVICEID_REGEX: &str = r"^[A-Za-z0-9\-:.+%_#*?!(),=@;$']{1,128}$";
const HOSTNAME_REGEX: &str = r"^[a-zA-Z0-9_\-\.]+$";

/// This is the default connection string
pub const DEFAULT_CONNECTION_STRING: &str = "<ADD DEVICE CONNECTION STRING HERE>";

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct ManualX509Auth {
    iothub_hostname: String,
    device_id: String,
    #[serde(with = "url_serde")]
    identity_cert: Url,
    #[serde(with = "url_serde")]
    identity_pk: Url,
}

impl ManualX509Auth {
    pub fn iothub_hostname(&self) -> &str {
        &self.iothub_hostname
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn identity_cert(&self) -> Result<PathBuf, Error> {
        get_path_from_uri(
            &self.identity_cert,
            "provisioning.authentication.identity_cert",
        )
    }

    pub fn identity_pk(&self) -> Result<PathBuf, Error> {
        get_path_from_uri(&self.identity_pk, "provisioning.authentication.identity_pk")
    }

    pub fn identity_pk_uri(&self) -> Result<&Url, Error> {
        if is_supported_uri(&self.identity_pk) {
            Ok(&self.identity_pk)
        } else {
            Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                self.identity_pk.to_string(),
                "provisioning.authentication.identity_pk",
            )))
        }
    }

    pub fn identity_cert_uri(&self) -> Result<&Url, Error> {
        if is_supported_uri(&self.identity_cert) {
            Ok(&self.identity_cert)
        } else {
            Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                self.identity_cert.to_string(),
                "provisioning.authentication.identity_cert",
            )))
        }
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct ManualDeviceConnectionString {
    device_connection_string: String,
}

impl ManualDeviceConnectionString {
    pub fn new(device_connection_string: String) -> Self {
        ManualDeviceConnectionString {
            device_connection_string,
        }
    }

    pub fn device_connection_string(&self) -> &str {
        &self.device_connection_string
    }

    pub fn parse_device_connection_string(&self) -> Result<(MemoryKey, String, String), Error> {
        if self.device_connection_string.is_empty() {
            return Err(Error::from(ErrorKind::ConnectionStringEmpty));
        }

        if self.device_connection_string == DEFAULT_CONNECTION_STRING {
            return Err(Error::from(ErrorKind::ConnectionStringNotConfigured(
                if cfg!(windows) {
                    "https://aka.ms/iot-edge-configure-windows"
                } else {
                    "https://aka.ms/iot-edge-configure-linux"
                },
            )));
        }

        let mut key = None;
        let mut device_id = None;
        let mut hub = None;

        let parts: Vec<&str> = self.device_connection_string.split(';').collect();
        for p in parts {
            let s: Vec<&str> = p.split('=').collect();
            match s[0] {
                SHAREDACCESSKEY_KEY => key = Some(s[1].to_string()),
                DEVICEID_KEY => device_id = Some(s[1].to_string()),
                HOSTNAME_KEY => hub = Some(s[1].to_string()),
                _ => (), // Ignore extraneous component in the connection string
            }
        }

        let key = key.ok_or(ErrorKind::ConnectionStringMissingRequiredParameter(
            SHAREDACCESSKEY_KEY,
        ))?;
        if key.is_empty() {
            return Err(Error::from(ErrorKind::ConnectionStringMalformedParameter(
                SHAREDACCESSKEY_KEY,
            )));
        }
        let key = MemoryKey::new(
            base64::decode(&key)
                .map_err(|_| ErrorKind::ConnectionStringMalformedParameter(SHAREDACCESSKEY_KEY))?,
        );

        let device_id =
            device_id.ok_or(ErrorKind::ConnectionStringMalformedParameter(DEVICEID_KEY))?;
        let device_id_regex =
            Regex::new(DEVICEID_REGEX).expect("This hard-coded regex is expected to be valid.");
        if !device_id_regex.is_match(&device_id) {
            return Err(Error::from(ErrorKind::ConnectionStringMalformedParameter(
                DEVICEID_KEY,
            )));
        }

        let hub = hub.ok_or(ErrorKind::ConnectionStringMissingRequiredParameter(
            HOSTNAME_KEY,
        ))?;
        let hub_regex =
            Regex::new(HOSTNAME_REGEX).expect("This hard-coded regex is expected to be valid.");
        if !hub_regex.is_match(&hub) {
            return Err(Error::from(ErrorKind::ConnectionStringMalformedParameter(
                HOSTNAME_KEY,
            )));
        }

        Ok((key, device_id.to_owned(), hub.to_owned()))
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(tag = "method")]
#[serde(rename_all = "lowercase")]
pub enum ManualAuthMethod {
    #[serde(rename = "device_connection_string")]
    DeviceConnectionString(ManualDeviceConnectionString),
    X509(ManualX509Auth),
}

#[derive(Clone, Debug, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct Manual {
    authentication: ManualAuthMethod,
}

impl<'de> serde::Deserialize<'de> for Manual {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, serde_derive::Deserialize)]
        struct Inner {
            #[serde(skip_serializing_if = "Option::is_none")]
            device_connection_string: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            authentication: Option<ManualAuthMethod>,
        }

        let value: Inner = serde::Deserialize::deserialize(deserializer)?;

        let authentication = match (value.device_connection_string, value.authentication) {
            (Some(_), Some(_)) => {
                return Err(serde::de::Error::custom(
                        "Only one of provisioning.device_connection_string or provisioning.authentication must be set in the config.yaml.",
                    ));
            }
            (Some(cs), None) => {
                ManualAuthMethod::DeviceConnectionString(ManualDeviceConnectionString::new(cs))
            }
            (None, Some(auth)) => auth,
            (None, None) => {
                return Err(serde::de::Error::custom(
                    "One of provisioning.device_connection_string or provisioning.authentication must be set in the config.yaml.",
                ));
            }
        };

        Ok(Manual { authentication })
    }
}

impl Manual {
    pub fn new(authentication: ManualAuthMethod) -> Self {
        Manual { authentication }
    }

    pub fn authentication_method(&self) -> &ManualAuthMethod {
        &self.authentication
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(tag = "method")]
#[serde(rename_all = "lowercase")]
pub enum AttestationMethod {
    Tpm(TpmAttestationInfo),
    #[serde(rename = "symmetric_key")]
    SymmetricKey(SymmetricKeyAttestationInfo),
    X509(X509AttestationInfo),
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct TpmAttestationInfo {
    registration_id: String,
}

impl TpmAttestationInfo {
    pub fn new(registration_id: String) -> Self {
        TpmAttestationInfo { registration_id }
    }

    pub fn registration_id(&self) -> &str {
        &self.registration_id
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct SymmetricKeyAttestationInfo {
    registration_id: String,
    symmetric_key: String,
}

impl SymmetricKeyAttestationInfo {
    pub fn registration_id(&self) -> &str {
        &self.registration_id
    }

    pub fn symmetric_key(&self) -> &str {
        &self.symmetric_key
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct X509AttestationInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    registration_id: Option<String>,
    #[serde(with = "url_serde")]
    identity_cert: Url,
    #[serde(with = "url_serde")]
    identity_pk: Url,
}

impl X509AttestationInfo {
    pub fn identity_cert(&self) -> Result<PathBuf, Error> {
        get_path_from_uri(
            &self.identity_cert,
            "provisioning.attestation.identity_cert",
        )
    }

    pub fn identity_pk(&self) -> Result<PathBuf, Error> {
        get_path_from_uri(&self.identity_pk, "provisioning.attestation.identity_pk")
    }

    pub fn identity_pk_uri(&self) -> Result<&Url, Error> {
        if is_supported_uri(&self.identity_pk) {
            Ok(&self.identity_pk)
        } else {
            Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                self.identity_pk.to_string(),
                "provisioning.attestation.identity_pk",
            )))
        }
    }

    pub fn identity_cert_uri(&self) -> Result<&Url, Error> {
        if is_supported_uri(&self.identity_cert) {
            Ok(&self.identity_cert)
        } else {
            Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                self.identity_cert.to_string(),
                "provisioning.attestation.identity_cert",
            )))
        }
    }

    pub fn registration_id(&self) -> Option<&str> {
        self.registration_id.as_ref().map(AsRef::as_ref)
    }
}

#[derive(Clone, Debug, serde_derive::Serialize)]
pub struct Dps {
    #[serde(with = "url_serde")]
    global_endpoint: Url,
    scope_id: String,
    attestation: AttestationMethod,
}

impl<'de> serde::Deserialize<'de> for Dps {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, serde_derive::Deserialize)]
        struct Inner {
            #[serde(with = "url_serde")]
            global_endpoint: Url,
            scope_id: String,
            registration_id: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            attestation: Option<AttestationMethod>,
        }

        let value: Inner = serde::Deserialize::deserialize(deserializer)?;

        let attestation = match (value.attestation, value.registration_id) {
            (Some(_att), Some(_)) => {
                return Err(serde::de::Error::custom(
                    "Provisioning registration_id has to be set only in attestation",
                ));
            }
            (Some(att), None) => att,
            (None, Some(reg_id)) => {
                AttestationMethod::Tpm(TpmAttestationInfo::new(reg_id.to_string()))
            }
            (None, None) => {
                return Err(serde::de::Error::custom(
                    "Provisioning registration_id has to be set",
                ));
            }
        };

        Ok(Dps {
            global_endpoint: value.global_endpoint,
            scope_id: value.scope_id,
            attestation,
        })
    }
}

impl Dps {
    pub fn global_endpoint(&self) -> &Url {
        &self.global_endpoint
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    pub fn attestation(&self) -> &AttestationMethod {
        &self.attestation
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct External {
    #[serde(with = "url_serde")]
    endpoint: Url,
}

impl External {
    pub fn new(endpoint: Url) -> Self {
        External { endpoint }
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(tag = "source")]
#[serde(rename_all = "lowercase")]
pub enum Provisioning {
    Manual(Box<Manual>),
    Dps(Box<Dps>),
    External(External),
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct Connect {
    #[serde(with = "url_serde")]
    workload_uri: Url,
    #[serde(with = "url_serde")]
    management_uri: Url,
}

impl Connect {
    pub fn workload_uri(&self) -> &Url {
        &self.workload_uri
    }

    pub fn management_uri(&self) -> &Url {
        &self.management_uri
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct Listen {
    #[serde(with = "url_serde")]
    workload_uri: Url,
    #[serde(with = "url_serde")]
    management_uri: Url,
}

impl Listen {
    pub fn workload_uri(&self) -> &Url {
        &self.workload_uri
    }

    pub fn management_uri(&self) -> &Url {
        &self.management_uri
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct Certificates {
    device_ca_cert: String,
    device_ca_pk: String,
    trusted_ca_certs: String,
}

fn is_supported_uri(uri: &Url) -> bool {
    if uri.scheme() == "file" && uri.port().is_none() && uri.query().is_none() {
        if let Some(host) = uri.host_str() {
            return host == "localhost";
        }
        return true;
    }
    false
}

fn get_path_from_uri(uri: &Url, setting_name: &'static str) -> Result<PathBuf, Error> {
    if is_supported_uri(&uri) {
        let path = uri
            .to_file_path()
            .map_err(|()| ErrorKind::InvalidSettingsUriFilePath(uri.to_string(), setting_name))?;
        Ok(path)
    } else {
        Err(Error::from(ErrorKind::UnsupportedSettingsFileUri(
            uri.to_string(),
            setting_name,
        )))
    }
}

fn convert_to_path(maybe_path: &str, setting_name: &'static str) -> Result<PathBuf, Error> {
    if let Ok(file_uri) = Url::from_file_path(maybe_path) {
        // maybe_path was specified as a valid absolute path not a URI
        get_path_from_uri(&file_uri, setting_name)
    } else {
        // maybe_path is a URI or a relative path
        if let Ok(uri) = Url::parse(maybe_path) {
            get_path_from_uri(&uri, setting_name)
        } else {
            Ok(PathBuf::from(maybe_path))
        }
    }
}

fn convert_to_uri(maybe_uri: &str, setting_name: &'static str) -> Result<Url, Error> {
    if let Ok(uri) = Url::parse(maybe_uri) {
        // maybe_uri was specified as a URI
        if is_supported_uri(&uri) {
            Ok(uri)
        } else {
            Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                maybe_uri.to_owned(),
                setting_name,
            )))
        }
    } else {
        // maybe_uri was specified as a valid path not a URI
        Url::from_file_path(maybe_uri)
            .map(|uri| {
                if is_supported_uri(&uri) {
                    Ok(uri)
                } else {
                    Err(Error::from(ErrorKind::UnsupportedSettingsUri(
                        maybe_uri.to_owned(),
                        setting_name,
                    )))
                }
            })
            .map_err(|()| ErrorKind::InvalidSettingsUri(maybe_uri.to_owned(), setting_name))?
    }
}

impl Certificates {
    pub fn device_ca_cert(&self) -> Result<PathBuf, Error> {
        convert_to_path(&self.device_ca_cert, "certificates.device_ca_cert")
    }

    pub fn device_ca_pk(&self) -> Result<PathBuf, Error> {
        convert_to_path(&self.device_ca_pk, "certificates.device_ca_pk")
    }

    pub fn trusted_ca_certs(&self) -> Result<PathBuf, Error> {
        convert_to_path(&self.trusted_ca_certs, "certificates.trusted_ca_certs")
    }

    pub fn device_ca_cert_uri(&self) -> Result<Url, Error> {
        convert_to_uri(&self.device_ca_cert, "certificates.device_ca_cert")
    }

    pub fn device_ca_pk_uri(&self) -> Result<Url, Error> {
        convert_to_uri(&self.device_ca_pk, "certificates.device_ca_pk")
    }

    pub fn trusted_ca_certs_uri(&self) -> Result<Url, Error> {
        convert_to_uri(&self.trusted_ca_certs, "certificates.trusted_ca_certs")
    }
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
#[serde(untagged)]
pub enum RetryLimit {
    Infinite,
    Num(u32),
}

impl RetryLimit {
    pub fn compare(&self, right: u32) -> Ordering {
        match self {
            RetryLimit::Infinite => Ordering::Greater,
            RetryLimit::Num(n) => n.cmp(&right),
        }
    }
}

impl Default for RetryLimit {
    fn default() -> Self {
        RetryLimit::Infinite
    }
}

#[derive(Clone, Debug, Default, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct WatchdogSettings {
    #[serde(default)]
    max_retries: RetryLimit,
}

impl WatchdogSettings {
    pub fn max_retries(&self) -> &RetryLimit {
        &self.max_retries
    }
}

pub trait RuntimeSettings {
    type Config;

    fn provisioning(&self) -> &Provisioning;
    fn agent(&self) -> &ModuleSpec<Self::Config>;
    fn agent_mut(&mut self) -> &mut ModuleSpec<Self::Config>;
    fn hostname(&self) -> &str;
    fn connect(&self) -> &Connect;
    fn listen(&self) -> &Listen;
    fn homedir(&self) -> &Path;
    fn certificates(&self) -> Option<&Certificates>;
    fn watchdog(&self) -> &WatchdogSettings;
}

#[derive(Clone, Debug, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct Settings<T> {
    provisioning: Provisioning,
    agent: ModuleSpec<T>,
    hostname: String,
    connect: Connect,
    listen: Listen,
    homedir: PathBuf,
    certificates: Option<Certificates>,
    #[serde(default)]
    watchdog: WatchdogSettings,
}

impl<T> RuntimeSettings for Settings<T>
where
    T: Clone,
{
    type Config = T;

    fn provisioning(&self) -> &Provisioning {
        &self.provisioning
    }

    fn agent(&self) -> &ModuleSpec<T> {
        &self.agent
    }

    fn agent_mut(&mut self) -> &mut ModuleSpec<T> {
        &mut self.agent
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }

    fn connect(&self) -> &Connect {
        &self.connect
    }

    fn listen(&self) -> &Listen {
        &self.listen
    }

    fn homedir(&self) -> &Path {
        &self.homedir
    }

    fn certificates(&self) -> Option<&Certificates> {
        self.certificates.as_ref()
    }

    fn watchdog(&self) -> &WatchdogSettings {
        &self.watchdog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_path() {
        if cfg!(windows) {
            assert_eq!(
                r"..\sample.txt",
                convert_to_path(r"..\sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );

            let expected_path = r"C:\temp\sample.txt";
            assert_eq!(
                expected_path,
                convert_to_path(r"C:\temp\sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file:///C:/temp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file://localhost/C:/temp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file://localhost/C:/temp/../temp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            // oddly this works because the host is null since local drive is specified
            assert_eq!(
                expected_path,
                convert_to_path("file://deadhost/C:/temp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            convert_to_path("file://deadhost/temp/sample.txt", "test")
                .expect_err("Non localhost host specified");
            convert_to_path("https:///C:/temp/sample.txt", "test")
                .expect_err("Non file scheme specified");
        } else {
            assert_eq!(
                "./sample.txt",
                convert_to_path("./sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );

            let expected_path = "/tmp/sample.txt";
            assert_eq!(
                expected_path,
                convert_to_path("/tmp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file:///tmp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file://localhost/tmp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            assert_eq!(
                expected_path,
                convert_to_path("file:///tmp/../tmp/sample.txt", "test")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            convert_to_path("file://deadhost/tmp/sample.txt", "test")
                .expect_err("Non localhost host specified");
            convert_to_path("https://localhost/tmp/sample.txt", "test")
                .expect_err("Non file scheme specified");
        }
    }

    #[test]
    fn test_convert_to_uri() {
        if cfg!(windows) {
            let expected_uri_str = "file:///C:/temp/sample.txt";
            let expected_uri = Url::parse(expected_uri_str).unwrap();

            assert_eq!(
                expected_uri,
                convert_to_uri("file:///C:/temp/sample.txt", "test").unwrap()
            );
            assert_eq!(
                expected_uri,
                convert_to_uri("file://localhost/C:/temp/sample.txt", "test").unwrap()
            );
            assert_eq!(
                expected_uri,
                convert_to_uri("file://localhost/C:/temp/../temp/sample.txt", "test").unwrap()
            );
            // oddly this works because the host is null since local drive is specified
            assert_eq!(
                expected_uri,
                convert_to_uri("file://deadhost/C:/temp/sample.txt", "test").unwrap()
            );
            convert_to_uri("file://deadhost/temp/sample.txt", "test")
                .expect_err("Non localhost host specified");
            convert_to_uri("file://deadhost/temp/sample.txt", "test")
                .expect_err("Non file scheme specified");
            convert_to_uri("../tmp/../tmp/sample.txt", "test")
                .expect_err("Non absolute path specified");
        } else {
            let expected_uri_str = "file:///tmp/sample.txt";
            let expected_uri = Url::parse(expected_uri_str).unwrap();

            assert_eq!(
                expected_uri,
                convert_to_uri("file:///tmp/sample.txt", "test").unwrap()
            );
            assert_eq!(
                expected_uri,
                convert_to_uri("file://localhost/tmp/sample.txt", "test").unwrap()
            );
            assert_eq!(
                expected_uri,
                convert_to_uri("file:///tmp/../tmp/sample.txt", "test").unwrap()
            );
            convert_to_uri("https://localhost/tmp/sample.txt", "test")
                .expect_err("Non absolute path specified");
            assert_eq!(
                expected_uri,
                convert_to_uri("/tmp/sample.txt", "test").unwrap()
            );
            convert_to_uri("../tmp/../tmp/sample.txt", "test")
                .expect_err("Non absolute path specified");
            convert_to_uri("file://deadhost/tmp/sample.txt", "test")
                .expect_err("Non localhost host specified");
        }
    }
}
