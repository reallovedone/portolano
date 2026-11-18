//! Kerberos for NLA (CredSSP / Negotiate).
//!
//! Three small pieces, all independent of the network so they can be tested on their own:
//!
//! - [`applies`]: when a connection signs in with Kerberos at all. Anything else keeps the
//!   NTLM-only behaviour of earlier versions, so a connection that worked keeps working;
//! - [`normalize_kdc`]: the optional "Kerberos KDC" typed by the user, turned into the URL
//!   sspi expects;
//! - [`TimeoutNetworkClient`]: the client that carries the requests to the KDC, with a
//!   deadline for each of them.
//!
//! # Why a fixed client host name
//!
//! sspi wants the name of the client computer for the Kerberos exchange. Portolano is not
//! joined to the domain and the value is only informational for the KDC, so a fixed
//! `PORTOLANO` is used: the real host name of the user's machine is not sent anywhere.
//!
//! # No secrets in logs
//!
//! Nothing here logs a user name, a password, a token or a KDC URL (a KDC proxy URL may carry
//! a query string).

use std::net::IpAddr;
use std::time::Duration;

use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::sspi::generator::NetworkRequest;
use ironrdp_connector::sspi::{Error as SspiError, ErrorKind as SspiErrorKind};
use ironrdp_connector::{ConnectorError, ConnectorErrorKind, ConnectorResult};
use ironrdp_tokio::NetworkClient;
use url::Url;

/// Client computer name sent to sspi (see the module documentation).
pub(crate) const CLIENT_HOSTNAME: &str = "PORTOLANO";

/// How long one request to the KDC may take. A DC that does not answer in this time is treated
/// as unreachable instead of keeping the user waiting for the whole login timeout.
pub(crate) const KDC_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

const DEFAULT_KDC_PORT: u16 = 88;

/// Whether the sign-in of this connection uses Kerberos.
///
/// Only when all of these hold:
///
/// - a credential is sent (NLA);
/// - the connection host is a name: Kerberos service names are built from the host name, an IP
///   literal never matches one;
/// - the realm is known: the user name is a UPN (`user@corp.example.com`) and the domain field
///   is empty, or the domain field holds a dotted name (`corp.example.com`) and the user name is
///   a plain one. A NetBIOS domain (`CORP`) does not name a realm.
///
/// Everything else, including `DOMAIN\user`, a UPN together with a filled domain field (it
/// would have to be turned into a mixed `DOMAIN\user@x` identity) and a short domain, keeps
/// NTLM exactly as before.
pub(crate) fn applies(
    host: &str,
    username: &str,
    domain: Option<&str>,
    has_credentials: bool,
) -> bool {
    if !has_credentials || is_ip_literal(host) {
        return false;
    }
    let username = username.trim();
    let domain = domain.map(str::trim).filter(|d| !d.is_empty());
    if username.contains('\\') {
        return false;
    }
    match (is_upn(username), domain) {
        (true, None) => true,
        (true, Some(_)) => false,
        (false, Some(domain)) => is_dotted_name(domain),
        (false, None) => false,
    }
}

fn is_ip_literal(host: &str) -> bool {
    let host = host.trim();
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    // A zone id (`fe80::1%en0`) is not accepted by `IpAddr`, but it is still a literal.
    let bare = bare.split('%').next().unwrap_or(bare);
    bare.parse::<IpAddr>().is_ok()
}

/// `user@realm.example`: one non-empty user part and a dotted realm.
fn is_upn(username: &str) -> bool {
    match username.rsplit_once('@') {
        Some((user, realm)) => !user.is_empty() && is_dotted_name(realm),
        None => false,
    }
}

fn is_dotted_name(name: &str) -> bool {
    let name = name.trim_matches('.');
    name.contains('.') && !name.contains(char::is_whitespace) && !name.contains("..")
}

/// Why a "Kerberos KDC" value was refused. The text is shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KdcError {
    #[error("The Kerberos KDC is not a valid address. Use a server name such as dc.example.com, optionally with a port, or an https:// KDC proxy address")]
    Invalid,
    #[error("The Kerberos KDC must use tcp://, udp:// or https://")]
    Scheme,
    #[error("The Kerberos KDC must not contain a user name or password")]
    Credentials,
}

/// Turns what the user typed into the KDC URL for sspi.
///
/// - empty: `None` (sspi finds the KDC through DNS);
/// - `dc.example.com`, `dc.example.com:88`, `10.0.0.5`, `[fe80::1]`: `tcp://` is assumed;
/// - `tcp://` and `udp://`: the port defaults to 88;
/// - `https://proxy/KdcProxy`: kept as it is (a KDC proxy, KKDCP). Plain `http://` is refused:
///   the sign-in request would travel unencrypted.
pub fn normalize_kdc(input: &str) -> Result<Option<String>, KdcError> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    if input.chars().any(char::is_whitespace) {
        return Err(KdcError::Invalid);
    }
    let text = if input.contains("://") {
        input.to_owned()
    } else {
        format!("tcp://{input}")
    };
    let mut url = Url::parse(&text).map_err(|_| KdcError::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(KdcError::Credentials);
    }
    match url.scheme() {
        "tcp" | "udp" => {
            if url.host_str().is_none_or(str::is_empty) {
                return Err(KdcError::Invalid);
            }
            if url.port().is_none() {
                url.set_port(Some(DEFAULT_KDC_PORT))
                    .map_err(|()| KdcError::Invalid)?;
            }
            // A KDC address is a host and a port: nothing else is meaningful.
            if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some()
            {
                return Err(KdcError::Invalid);
            }
            url.set_path("");
        }
        "https" => {
            if url.host_str().is_none_or(str::is_empty) {
                return Err(KdcError::Invalid);
            }
        }
        _ => return Err(KdcError::Scheme),
    }
    Ok(Some(url.to_string()))
}

/// The Kerberos configuration of a connection, or `None` when it signs in with NTLM only.
/// `kdc` is the value typed by the user (not yet normalised).
pub(crate) fn config_for(
    host: &str,
    username: &str,
    domain: Option<&str>,
    has_credentials: bool,
    kdc: Option<&str>,
) -> Result<Option<KerberosConfig>, crate::RdpError> {
    if !applies(host, username, domain, has_credentials) {
        return Ok(None);
    }
    let kdc_url = kdc
        .map(normalize_kdc)
        .transpose()
        .map_err(|e| crate::RdpError::InvalidInput(e.to_string()))?
        .flatten();
    KerberosConfig::new(kdc_url, CLIENT_HOSTNAME.to_owned())
        .map(Some)
        .map_err(|_| crate::RdpError::InvalidInput(KdcError::Invalid.to_string()))
}

/// Puts a deadline on every request to the KDC and reports any failure as "unreachable".
///
/// A timeout is reported as sspi's "no authenticating authority" error (the same one the stock
/// client uses when a KDC cannot be reached), wrapped as the source of the connector error, so
/// [`crate::error::from_connector`] shows the same message for both.
pub(crate) struct TimeoutNetworkClient<N> {
    inner: N,
    timeout: Duration,
}

impl<N> TimeoutNetworkClient<N> {
    pub(crate) fn new(inner: N, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}

impl<N: NetworkClient> NetworkClient for TimeoutNetworkClient<N> {
    async fn send(&mut self, network_request: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
        match tokio::time::timeout(self.timeout, self.inner.send(network_request)).await {
            Ok(Ok(reply)) => Ok(reply),
            // Whatever went wrong (refused, reset, bad HTTP status, timeout) the KDC cannot be
            // used. The original error is dropped: it may quote the KDC address.
            Ok(Err(_)) => Err(unreachable_error("the KDC request failed")),
            Err(_) => Err(unreachable_error("the KDC did not answer in time")),
        }
    }
}

/// A connector error that carries sspi's "KDC unreachable" kind.
pub(crate) fn unreachable_error(reason: &str) -> ConnectorError {
    ConnectorError::new("KDC request", ConnectorErrorKind::Custom).with_source(SspiError::new(
        SspiErrorKind::NoAuthenticatingAuthority,
        reason,
    ))
}

#[cfg(test)]
mod tests {
    use ironrdp_connector::sspi::network_client::NetworkProtocol;

    use super::*;

    fn upn_case(host: &str, user: &str, domain: Option<&str>) -> bool {
        applies(host, user, domain, true)
    }

    #[test]
    fn a_name_and_a_upn_use_kerberos() {
        assert!(upn_case(
            "srv.corp.example.com",
            "ann@corp.example.com",
            None
        ));
        assert!(upn_case("srv", "ann@corp.example.com", Some("  ")));
    }

    #[test]
    fn a_short_server_name_is_still_a_name() {
        assert!(upn_case("srv01", "ann@corp.example.com", None));
    }

    #[test]
    fn a_dotted_domain_with_a_plain_user_uses_kerberos() {
        assert!(upn_case(
            "srv.corp.example.com",
            "ann",
            Some("corp.example.com")
        ));
    }

    #[test]
    fn an_ip_literal_keeps_ntlm() {
        assert!(!upn_case("10.0.0.5", "ann@corp.example.com", None));
        assert!(!upn_case("::1", "ann@corp.example.com", None));
        assert!(!upn_case("[fe80::1]", "ann@corp.example.com", None));
        assert!(!upn_case("fe80::1%en0", "ann@corp.example.com", None));
        assert!(!upn_case("2001:db8::7", "ann", Some("corp.example.com")));
    }

    #[test]
    fn a_netbios_domain_or_no_realm_keeps_ntlm() {
        assert!(!upn_case("srv.corp.example.com", "ann", Some("CORP")));
        assert!(!upn_case("srv.corp.example.com", "ann", None));
        assert!(!upn_case("srv.corp.example.com", "ann@corp", None));
        assert!(!upn_case("srv.corp.example.com", "@corp.example.com", None));
        assert!(!upn_case("srv.corp.example.com", "ann", Some("a..b")));
    }

    #[test]
    fn a_backslash_user_keeps_ntlm_whatever_the_domain() {
        assert!(!upn_case("srv.corp.example.com", "CORP\\ann", None));
        assert!(!upn_case(
            "srv.corp.example.com",
            "CORP\\ann",
            Some("corp.example.com")
        ));
        assert!(!upn_case(
            "srv.corp.example.com",
            "corp.example.com\\ann",
            None
        ));
    }

    #[test]
    fn a_upn_together_with_a_domain_keeps_ntlm() {
        assert!(!upn_case(
            "srv.corp.example.com",
            "ann@corp.example.com",
            Some("corp.example.com")
        ));
        assert!(!upn_case(
            "srv.corp.example.com",
            "ann@corp.example.com",
            Some("CORP")
        ));
    }

    #[test]
    fn without_a_credential_there_is_no_kerberos() {
        assert!(!applies(
            "srv.corp.example.com",
            "ann@corp.example.com",
            None,
            false
        ));
    }

    #[test]
    fn kdc_forms_are_normalised() {
        let cases = [
            ("dc.example.com", "tcp://dc.example.com:88"),
            ("  dc.example.com  ", "tcp://dc.example.com:88"),
            ("dc.example.com:88", "tcp://dc.example.com:88"),
            ("dc.example.com:8888", "tcp://dc.example.com:8888"),
            ("10.0.0.5", "tcp://10.0.0.5:88"),
            ("[fe80::1]", "tcp://[fe80::1]:88"),
            ("tcp://dc.example.com", "tcp://dc.example.com:88"),
            ("tcp://dc.example.com:89", "tcp://dc.example.com:89"),
            ("TCP://dc.example.com", "tcp://dc.example.com:88"),
            ("udp://10.0.0.5", "udp://10.0.0.5:88"),
            ("udp://10.0.0.5:99/", "udp://10.0.0.5:99"),
            (
                "https://proxy.example.com/KdcProxy",
                "https://proxy.example.com/KdcProxy",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_kdc(input),
                Ok(Some(expected.to_owned())),
                "{input}"
            );
        }
    }

    #[test]
    fn an_empty_kdc_means_dns_discovery() {
        assert_eq!(normalize_kdc(""), Ok(None));
        assert_eq!(normalize_kdc("   "), Ok(None));
    }

    #[test]
    fn invalid_kdc_values_are_errors_not_crashes() {
        for input in [
            "dc example.com",
            "tcp://",
            "tcp:// ",
            "dc.example.com:notaport",
            "dc.example.com:70000",
            "tcp://dc.example.com/path",
            "tcp://dc.example.com?x=1",
            "https://",
            "://dc",
            "\u{0}",
        ] {
            assert_eq!(normalize_kdc(input), Err(KdcError::Invalid), "{input:?}");
        }
        for input in ["http://proxy.example.com/KdcProxy", "ftp://dc", "ldap://dc"] {
            assert_eq!(normalize_kdc(input), Err(KdcError::Scheme), "{input}");
        }
        assert_eq!(
            normalize_kdc("tcp://ann:secret@dc.example.com"),
            Err(KdcError::Credentials)
        );
        assert_eq!(
            normalize_kdc("https://ann@proxy.example.com/KdcProxy"),
            Err(KdcError::Credentials)
        );
    }

    #[test]
    fn the_error_text_never_repeats_the_input() {
        let secret = "tcp://ann:hunter2@dc.example.com";
        let err = normalize_kdc(secret).expect_err("rejected");
        assert!(!err.to_string().contains("hunter2"));
    }

    #[test]
    fn config_is_built_only_when_kerberos_applies() {
        let none =
            config_for("10.0.0.5", "ann@corp.example.com", None, true, Some("dc")).expect("ok");
        assert!(none.is_none());
        let cfg = config_for(
            "srv.corp.example.com",
            "ann@corp.example.com",
            None,
            true,
            None,
        )
        .expect("ok")
        .expect("kerberos");
        assert!(cfg.kdc_proxy_url.is_none());
        assert_eq!(cfg.hostname, CLIENT_HOSTNAME);
        let cfg = config_for(
            "srv.corp.example.com",
            "ann@corp.example.com",
            None,
            true,
            Some("dc.corp.example.com"),
        )
        .expect("ok")
        .expect("kerberos");
        assert_eq!(
            cfg.kdc_proxy_url.map(|u| u.to_string()).as_deref(),
            Some("tcp://dc.corp.example.com:88")
        );
    }

    #[test]
    fn an_invalid_kdc_is_reported_when_kerberos_applies_and_ignored_otherwise() {
        let err = config_for(
            "srv.corp.example.com",
            "ann@corp.example.com",
            None,
            true,
            Some("bad value"),
        )
        .expect_err("invalid");
        assert!(matches!(err, crate::RdpError::InvalidInput(_)));
        assert!(config_for("10.0.0.5", "ann", None, true, Some("bad value"))
            .expect("ignored")
            .is_none());
    }

    fn request() -> NetworkRequest {
        NetworkRequest {
            protocol: NetworkProtocol::Tcp,
            url: Url::parse("tcp://dc.example.com:88").expect("url"),
            data: vec![1, 2, 3],
        }
    }

    struct Fake {
        delay: Duration,
        reply: Vec<u8>,
        calls: usize,
    }

    impl NetworkClient for Fake {
        async fn send(&mut self, network_request: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
            self.calls += 1;
            assert_eq!(network_request.data, [1, 2, 3]);
            tokio::time::sleep(self.delay).await;
            Ok(self.reply.clone())
        }
    }

    #[tokio::test]
    async fn a_prompt_answer_goes_through_unchanged() {
        let fake = Fake {
            delay: Duration::from_millis(1),
            reply: vec![9, 9],
            calls: 0,
        };
        let mut client = TimeoutNetworkClient::new(fake, Duration::from_secs(5));
        let reply = client.send(&request()).await.expect("answer");
        assert_eq!(reply, [9, 9]);
        assert_eq!(client.inner.calls, 1);
    }

    #[tokio::test]
    async fn a_slow_kdc_times_out_as_unreachable() {
        let fake = Fake {
            delay: Duration::from_secs(30),
            reply: vec![],
            calls: 0,
        };
        let mut client = TimeoutNetworkClient::new(fake, Duration::from_millis(30));
        let started = std::time::Instant::now();
        let err = client.send(&request()).await.expect_err("timeout");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            crate::error::sspi_kind_in_chain(&err),
            Some(SspiErrorKind::NoAuthenticatingAuthority)
        );
        let text = err.report().to_string();
        assert!(!text.contains("dc.example.com"), "{text}");
    }

    struct Failing;

    impl NetworkClient for Failing {
        async fn send(&mut self, _: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
            Err(
                ConnectorError::new("inner", ConnectorErrorKind::General).with_source(
                    SspiError::new(
                        SspiErrorKind::InternalError,
                        "HTTP 500 from proxy.secret.example",
                    ),
                ),
            )
        }
    }

    #[tokio::test]
    async fn any_inner_error_becomes_an_unreachable_kdc_without_its_text() {
        let mut client = TimeoutNetworkClient::new(Failing, Duration::from_secs(5));
        let err = client.send(&request()).await.expect_err("failed");
        assert_eq!(
            crate::error::sspi_kind_in_chain(&err),
            Some(SspiErrorKind::NoAuthenticatingAuthority)
        );
        assert!(!err.report().to_string().contains("secret"));
    }

    #[test]
    fn the_kdc_timeout_is_a_few_seconds() {
        assert_eq!(KDC_REQUEST_TIMEOUT, Duration::from_secs(5));
    }
}
