//! The selected PostgreSQL endpoint and the redactor derived from it.
//!
//! [`Endpoint`] owns the one place a raw connection URL exists in this crate.
//! Its `Debug`/`Display` never render it; the raw value is only reachable
//! through [`Endpoint::expose`], which the runner uses to hand the URL to a
//! child process through the environment.

use std::fmt;

use sqlx::postgres::{PgConnectOptions, PgSslMode};

use super::MalformedTarget;

const REDACTED: &str = "<redacted>";
const AUTHORITY_TERMINATORS: &[char] = &['/', '?', '#', '"', '\'', ')', '<', '>', ','];

/// PostgreSQL host:port/database the operator designated for backend tests.
pub struct Endpoint {
    raw: String,
    child_url: String,
    options: PgConnectOptions,
    host: String,
    port: u16,
    username: String,
    database: String,
}

impl Endpoint {
    /// Validate `raw` as a PostgreSQL test target.
    ///
    /// # Errors
    ///
    /// Returns the sanitized [`MalformedTarget`] category describing why the
    /// value cannot designate exactly one database.
    pub fn parse(raw: &str) -> Result<Self, MalformedTarget> {
        let trimmed = raw.trim();
        let scheme_end = trimmed
            .find("://")
            .ok_or(MalformedTarget::UnsupportedScheme)?;
        let scheme = trimmed[..scheme_end].to_ascii_lowercase();
        if scheme != "postgres" && scheme != "postgresql" {
            return Err(MalformedTarget::UnsupportedScheme);
        }
        if trimmed.contains('#') {
            return Err(MalformedTarget::FragmentNotAllowed);
        }

        let rest = &trimmed[scheme_end + 3..];
        let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, remainder) = rest.split_at(authority_end);
        let (userinfo, host_and_port) = authority
            .rsplit_once('@')
            .map_or(("", authority), |(userinfo, hp)| (userinfo, hp));
        let authority_port = if host_and_port.is_empty() {
            if !has_query_parameter(trimmed, "host") && !has_query_parameter(trimmed, "hostaddr") {
                return Err(MalformedTarget::MissingHost);
            }
            None
        } else {
            let (authority_host, port) = split_host_port(host_and_port)?;
            if authority_host.is_empty()
                && !has_query_parameter(trimmed, "host")
                && !has_query_parameter(trimmed, "hostaddr")
            {
                return Err(MalformedTarget::MissingHost);
            }
            port
        };

        // Do not allow libpq environment variables or the OS account to supply
        // identity fields omitted from the selected URL. Query options count as
        // explicit fields because sqlx applies them to these same options.
        let authority_username = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
        if authority_username.is_empty() && !has_query_parameter(trimmed, "user") {
            return Err(MalformedTarget::MissingUsername);
        }

        let path = remainder.split('?').next().unwrap_or_default();
        let path_database = path.strip_prefix('/').unwrap_or_default();
        let has_database_override = has_query_parameter(trimmed, "dbname");
        if (path_database.is_empty() || path_database.contains('/')) && !has_database_override {
            return Err(MalformedTarget::MissingDatabase);
        }

        let has_explicit_port = authority_port.is_some() || has_query_parameter(trimmed, "port");
        let mut options = trimmed
            .parse::<PgConnectOptions>()
            .map_err(|_| MalformedTarget::Unparseable)?;
        if !has_explicit_port {
            options = options.port(5432);
        }

        let database = options
            .get_database()
            .filter(|database| !database.is_empty())
            .ok_or(MalformedTarget::MissingDatabase)?
            .to_owned();
        let username = options.get_username().to_owned();
        if username.is_empty() {
            return Err(MalformedTarget::MissingUsername);
        }
        let host = options
            .get_socket()
            .map(|socket| socket.to_string_lossy().into_owned())
            .unwrap_or_else(|| options.get_host().to_owned());
        if host.is_empty() {
            return Err(MalformedTarget::MissingHost);
        }
        let port = options.get_port();

        let child_url = if has_explicit_port {
            trimmed.to_owned()
        } else {
            append_query_option(trimmed, "port=5432")
        };

        Ok(Self {
            raw: trimmed.to_owned(),
            child_url,
            options,
            host,
            port,
            username,
            database,
        })
    }

    /// Build sqlx connection options, preserving every effective query option
    /// carried by the selected URL.
    ///
    pub fn connect_options(&self) -> Result<PgConnectOptions, MalformedTarget> {
        Ok(self.options.clone())
    }

    /// The effective database selected after URL query options are applied.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Opaque identity of this effective endpoint; nothing in it is reversible
    /// to a host, port, role, or database name.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut seed = String::with_capacity(self.host.len() + self.database.len() + 16);
        seed.push_str(&self.host);
        seed.push('\0');
        seed.push_str(&self.port.to_string());
        seed.push('\0');
        seed.push_str(&self.database);
        seed.push('\0');
        seed.push_str(&self.username);
        format!("{:016x}", fnv1a64(seed.as_bytes()))
    }

    /// Whether the URL asks for a TLS-protected session.
    #[must_use]
    pub fn ssl_mode(&self) -> PgSslMode {
        self.options.get_ssl_mode()
    }

    /// The normalized URL for a child. Callers must place it in a child
    /// environment variable and never render it.
    pub fn expose(&self) -> &str {
        &self.child_url
    }

    /// A redactor bound to this endpoint's secret material.
    #[must_use]
    pub fn redactor(&self) -> Redactor {
        Redactor::with_secrets([self.raw.as_str(), self.child_url.as_str()])
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("fingerprint", &self.fingerprint())
            .field("ssl_mode", &self.ssl_mode())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<test database {}>", self.fingerprint())
    }
}

fn append_query_option(raw: &str, option: &str) -> String {
    match raw.rfind('?') {
        None => format!("{raw}?{option}"),
        Some(index) if index + 1 == raw.len() => format!("{raw}{option}"),
        Some(_) => format!("{raw}&{option}"),
    }
}

fn split_host_port(value: &str) -> Result<(String, Option<u16>), MalformedTarget> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, tail) = rest.split_once(']').ok_or(MalformedTarget::MissingHost)?;
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port.parse().map_err(|_| MalformedTarget::MissingHost)?),
            None => None,
        };
        return Ok((format!("[{host}]"), port));
    }
    match value.rsplit_once(':') {
        Some((host, port)) => Ok((
            host.to_owned(),
            Some(port.parse().map_err(|_| MalformedTarget::MissingHost)?),
        )),
        None => Ok((value.to_owned(), None)),
    }
}

fn has_query_parameter(raw: &str, expected: &str) -> bool {
    raw.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            decode_query_component(key).as_deref() == Some(expected)
        })
    })
}

fn decode_query_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1])?;
                let low = hex_value(bytes[index + 2])?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' => return None,
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Removes known connection secrets and masks anything that still looks like
/// a PostgreSQL URL with inline credentials.
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    /// Build a redactor for an explicit list of literal secrets.
    #[must_use]
    pub fn with_secrets<'a>(secrets: impl IntoIterator<Item = &'a str>) -> Self {
        let mut collected: Vec<String> = secrets
            .into_iter()
            .filter(|secret| !secret.is_empty())
            .map(str::to_owned)
            .collect();
        collected.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self { secrets: collected }
    }

    /// Replace every known secret and mask residual `scheme://user:pass@`
    /// sequences.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let mut output = text.to_owned();
        for secret in &self.secrets {
            if output.contains(secret.as_str()) {
                output = output.replace(secret.as_str(), "<database-url-redacted>");
            }
        }
        mask_inline_credentials(&output)
    }
}

/// Scan for `postgres://`/`postgresql://` authorities that carry a password and
/// replace the password with a placeholder.
fn mask_inline_credentials(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(relative) = find_scheme(&text[cursor..]) else {
            break;
        };
        let index = cursor + relative;
        let authority_start = index + scheme_length(&text[index..]);
        let tail = &text[authority_start..];
        let authority_len = tail
            .find(char::is_whitespace)
            .or_else(|| tail.find(AUTHORITY_TERMINATORS))
            .unwrap_or(tail.len());
        output.push_str(&text[cursor..authority_start]);
        output.push_str(&mask_authority(&tail[..authority_len]));
        cursor = authority_start + authority_len;
    }
    output.push_str(&text[cursor..]);
    output
}

fn mask_authority(authority: &str) -> String {
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return authority.to_owned();
    };
    let Some((user, _password)) = userinfo.split_once(':') else {
        return authority.to_owned();
    };
    format!("{user}:{REDACTED}@{host}")
}

fn find_scheme(text: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(relative) = text[search_from..].find("postgres") {
        let index = search_from + relative;
        let tail = &text[index + "postgres".len()..];
        if tail.starts_with("ql://") || tail.starts_with("://") {
            return Some(index);
        }
        search_from = index + "postgres".len();
    }
    None
}

fn scheme_length(scheme_and_rest: &str) -> usize {
    if scheme_and_rest.starts_with("postgresql://") {
        "postgresql://".len()
    } else {
        "postgres://".len()
    }
}
