//! Whether the product's database answers: one TCP connection to the host
//! and port of its `DATABASE_URL`, nothing logged in, nothing read.
//!
//! The watch asks it once a tick: a code session or a repair started while
//! it is down runs its tests without a database — the ones that need it
//! skip in silence locally, then fail in CI (`ForgeDesMondes`, 2026-10-19/20:
//! Docker stopped all night, 7 of 20 red `rust` checks).

use std::time::Duration;

/// How long the connection may take before the database counts as down.
const CONNECT_WITHIN: Duration = Duration::from_secs(2);

/// The `host:port` a PostgreSQL URL names (`postgres://user:pass@host:5432/db`,
/// `postgresql://host/db` on the default port); `None` for what is no such URL.
#[must_use]
pub fn address_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let authority = rest.split(['/', '?']).next()?;
    let host_port = authority.rsplit('@').next()?;
    if host_port.is_empty() {
        return None;
    }
    Some(
        if host_port
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            host_port.to_string()
        } else {
            format!("{host_port}:5432")
        },
    )
}

/// Whether the database `url` names accepts a connection: `None` when the
/// URL names no PostgreSQL address (nothing to check, nothing held).
pub async fn answers(url: &str) -> Option<bool> {
    let address = address_of(url)?;
    Some(matches!(
        tokio::time::timeout(CONNECT_WITHIN, tokio::net::TcpStream::connect(&address)).await,
        Ok(Ok(_))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_read_off_the_url() {
        assert_eq!(
            address_of("postgres://app:secret@localhost:5433/forgedesmondes").as_deref(),
            Some("localhost:5433")
        );
        assert_eq!(
            address_of("postgresql://db.local/app").as_deref(),
            Some("db.local:5432")
        );
        assert_eq!(
            address_of("postgres://u@h:5432/d?sslmode=disable").as_deref(),
            Some("h:5432")
        );
        assert_eq!(address_of("mysql://h/d"), None);
        assert_eq!(address_of("postgres:///d"), None);
        assert_eq!(address_of(""), None);
    }

    #[tokio::test]
    async fn a_closed_port_does_not_answer_and_an_open_one_does() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        assert_eq!(
            answers(&format!("postgres://u:p@127.0.0.1:{port}/d")).await,
            Some(true)
        );
        // Not the port just released: a test beside this one may bind it
        // again first. Port 1 (tcpmux) is privileged and served by nothing
        // on a developer machine or a CI runner.
        drop(listener);
        assert_eq!(answers("postgres://u:p@127.0.0.1:1/d").await, Some(false));
        assert_eq!(answers("not a url").await, None);
    }
}
