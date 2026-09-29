//! steller as a cache, over a hand-written RESP client.
//!
//! steller is the Redis-compatible server at <https://github.com/scadoshi/steller>.
//! The client speaks plain RESP2 with no handshake, so it works against Redis too.

/// One connection and a request over it.
pub mod connection;

/// The wire codec.
pub mod resp;

use crate::domain::stats::{
    models::{cache_key::CacheKey, errors::CacheError},
    ports::StatsCache,
};
use connection::{Connection, ConnectionError};
use resp::Reply;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::Mutex;

/// Deadline for connecting, and separately for each command and its reply.
const DEADLINE: Duration = Duration::from_millis(500);

/// The address is not on the loopback interface.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "steller address {0} is not loopback: steller has no AUTH, no TLS and no ACL, so it is only safe to reach on 127.0.0.1 or ::1"
)]
pub struct NotLoopback(pub SocketAddr);

/// A cache backed by one connection to steller.
///
/// Commands are serialized behind a lock and the connection is opened on first use.
/// A command that fails or is rejected drops the connection, and the next one
/// reconnects.
#[derive(Debug, Clone)]
pub struct StellerCache {
    address: SocketAddr,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl StellerCache {
    /// A cache over the server at `address`, which must be loopback.
    pub fn new(address: SocketAddr) -> Result<Self, NotLoopback> {
        if !address.ip().is_loopback() {
            return Err(NotLoopback(address));
        }
        Ok(Self {
            address,
            connection: Arc::new(Mutex::new(None)),
        })
    }

    /// Sends one command and returns its reply.
    ///
    /// A connection left over from an earlier command may have been closed by the
    /// server since, which only shows when it is used. So a failure on a reused
    /// connection is retried once on a new one. Every command here is idempotent.
    async fn command(&self, parts: &[&[u8]]) -> Result<Reply, CacheError> {
        let command = resp::encode(parts);
        let mut slot = self.connection.lock().await;

        if let Some(mut reused) = slot.take() {
            match reused.roundtrip(&command, DEADLINE).await {
                Ok(reply) => return keep_unless_rejected(&mut slot, reused, reply),
                Err(error) => tracing::debug!("steller connection dropped, reconnecting: {error}"),
            }
        }

        let mut opened = Connection::open(self.address, DEADLINE)
            .await
            .map_err(unavailable)?;
        let reply = opened
            .roundtrip(&command, DEADLINE)
            .await
            .map_err(unavailable)?;
        keep_unless_rejected(&mut slot, opened, reply)
    }
}

/// Puts `connection` back for the next command, unless the reply is an error.
///
/// A server that rejects a command partway through reading it goes on to read the
/// rest as new commands and answers each. Those answers would be taken for the
/// replies to whatever is sent next, so the connection is closed.
fn keep_unless_rejected(
    slot: &mut Option<Connection>,
    connection: Connection,
    reply: Reply,
) -> Result<Reply, CacheError> {
    match reply {
        Reply::Error(message) => Err(CacheError(anyhow::anyhow!(
            "steller rejected the command: {}",
            String::from_utf8_lossy(&message)
        ))),
        reply => {
            *slot = Some(connection);
            Ok(reply)
        }
    }
}

fn unavailable(error: ConnectionError) -> CacheError {
    CacheError(anyhow::Error::new(error).context("steller"))
}

fn unexpected(command: &str, reply: &Reply) -> CacheError {
    CacheError(anyhow::anyhow!("unexpected reply to {command}: {reply:?}"))
}

impl StatsCache for StellerCache {
    fn backend(&self) -> &'static str {
        "steller"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError> {
        match self.command(&[b"GET", key.as_bytes()]).await? {
            Reply::Bulk(value) => Ok(Some(value)),
            Reply::Null => Ok(None),
            other => Err(unexpected("GET", &other)),
        }
    }

    async fn set(&self, key: &CacheKey, value: &[u8], retain: Duration) -> Result<(), CacheError> {
        // A zero `PX` is a syntax error in Redis, so the shortest window is 1ms.
        let millis = retain.as_millis().max(1).to_string();
        let reply = self
            .command(&[b"SET", key.as_bytes(), value, b"PX", millis.as_bytes()])
            .await?;
        match reply {
            Reply::Simple(ok) if ok == b"OK" => Ok(()),
            other => Err(unexpected("SET", &other)),
        }
    }

    async fn ping(&self) -> Result<(), CacheError> {
        match self.command(&[b"PING"]).await? {
            Reply::Simple(pong) if pong == b"PONG" => Ok(()),
            other => Err(unexpected("PING", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    // The fake server below is test code that clippy.toml's in-tests allowances do
    // not reach, since its helpers are not marked `#[test]`.
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;
    use crate::test_support::repo;
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicU32, Ordering},
        time::Instant,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    fn key() -> CacheKey {
        CacheKey::for_repo(&repo("a/b"))
    }

    /// How the fake server treats a connection.
    #[derive(Clone, Copy)]
    enum Behavior {
        /// Answers like a key-value server.
        Serve,
        /// Answers one byte at a time, so every reply arrives split.
        Trickle,
        /// Answers one command, then closes the connection.
        CloseAfterOne,
        /// Accepts and never answers.
        Silent,
        /// Answers every command with an error reply.
        Refuse,
        /// Rejects `SET` and follows the rejection with more error replies, as a
        /// server does after losing its place in the stream. Serves everything else.
        RejectThenFlood,
    }

    /// A RESP server small enough to read in one sitting. Returns its address and a
    /// count of the connections it has accepted.
    async fn fake_server(behavior: Behavior) -> (SocketAddr, Arc<AtomicU32>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicU32::new(0));
        let store = Arc::new(Mutex::new(HashMap::<Vec<u8>, Vec<u8>>::new()));
        let counter = Arc::clone(&accepted);
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(serve(stream, behavior, Arc::clone(&store)));
            }
        });
        (address, accepted)
    }

    async fn serve(
        mut stream: TcpStream,
        behavior: Behavior,
        store: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
    ) {
        let mut buf = Vec::new();
        loop {
            let Some(parts) = read_command(&mut stream, &mut buf).await else {
                return;
            };
            let reply = match behavior {
                Behavior::Silent => {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    return;
                }
                Behavior::Refuse => b"-ERR refused\r\n".to_vec(),
                Behavior::RejectThenFlood if parts[0].eq_ignore_ascii_case(b"SET") => {
                    b"-ERR missing crlf terminator\r\n-ERR unknown sigil\r\n-ERR unknown sigil\r\n"
                        .to_vec()
                }
                _ => answer(&parts, &store).await,
            };
            match behavior {
                Behavior::Trickle => {
                    for byte in reply {
                        stream.write_all(&[byte]).await.unwrap();
                        stream.flush().await.unwrap();
                        tokio::task::yield_now().await;
                    }
                }
                _ => stream.write_all(&reply).await.unwrap(),
            }
            if matches!(behavior, Behavior::CloseAfterOne) {
                return;
            }
        }
    }

    async fn answer(parts: &[Vec<u8>], store: &Mutex<HashMap<Vec<u8>, Vec<u8>>>) -> Vec<u8> {
        match parts[0].to_ascii_uppercase().as_slice() {
            b"PING" => b"+PONG\r\n".to_vec(),
            b"SET" => {
                store
                    .lock()
                    .await
                    .insert(parts[1].clone(), parts[2].clone());
                b"+OK\r\n".to_vec()
            }
            b"GET" => match store.lock().await.get(&parts[1]) {
                Some(value) => {
                    let mut reply = format!("${}\r\n", value.len()).into_bytes();
                    reply.extend_from_slice(value);
                    reply.extend_from_slice(b"\r\n");
                    reply
                }
                None => b"$-1\r\n".to_vec(),
            },
            _ => b"-ERR unknown command\r\n".to_vec(),
        }
    }

    /// Reads one array of bulk strings, or `None` when the client hangs up.
    async fn read_command(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Option<Vec<Vec<u8>>> {
        loop {
            if let Some((parts, used)) = parse_command(buf) {
                buf.drain(..used);
                return Some(parts);
            }
            if stream.read_buf(buf).await.ok()? == 0 {
                return None;
            }
        }
    }

    fn parse_command(buf: &[u8]) -> Option<(Vec<Vec<u8>>, usize)> {
        let mut at = 0;
        let line = |at: &mut usize| -> Option<Vec<u8>> {
            let end = buf[*at..].windows(2).position(|pair| pair == b"\r\n")?;
            let line = buf[*at..*at + end].to_vec();
            *at += end + 2;
            Some(line)
        };
        let header = line(&mut at)?;
        let count: usize = std::str::from_utf8(&header[1..]).ok()?.parse().ok()?;
        let mut parts = Vec::new();
        for _ in 0..count {
            let header = line(&mut at)?;
            let len: usize = std::str::from_utf8(&header[1..]).ok()?.parse().ok()?;
            let part = buf.get(at..at + len)?.to_vec();
            buf.get(at + len..at + len + 2)?;
            at += len + 2;
            parts.push(part);
        }
        Some((parts, at))
    }

    #[test]
    fn refuses_an_address_that_is_not_loopback() {
        for address in ["10.0.0.5:3000", "0.0.0.0:3000", "192.168.1.2:3000"] {
            let address: SocketAddr = address.parse().unwrap();
            assert_eq!(
                StellerCache::new(address).unwrap_err(),
                NotLoopback(address)
            );
        }
        assert!(StellerCache::new("127.0.0.1:3000".parse().unwrap()).is_ok());
        assert!(StellerCache::new("[::1]:3000".parse().unwrap()).is_ok());
    }

    #[tokio::test]
    async fn set_then_get_round_trips_binary_bytes() {
        let (address, _) = fake_server(Behavior::Serve).await;
        let cache = StellerCache::new(address).unwrap();
        let value = b"line one\r\nline two\x00\xff";
        cache
            .set(&key(), value, Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            cache.get(&key()).await.unwrap().as_deref(),
            Some(&value[..])
        );
    }

    #[tokio::test]
    async fn an_absent_key_is_none() {
        let (address, _) = fake_server(Behavior::Serve).await;
        let cache = StellerCache::new(address).unwrap();
        assert_eq!(cache.get(&key()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn ping_answers() {
        let (address, _) = fake_server(Behavior::Serve).await;
        assert!(StellerCache::new(address).unwrap().ping().await.is_ok());
    }

    #[tokio::test]
    async fn a_reply_split_at_every_byte_is_reassembled() {
        let (address, _) = fake_server(Behavior::Trickle).await;
        let cache = StellerCache::new(address).unwrap();
        let value = b"hello\r\nworld";
        cache
            .set(&key(), value, Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            cache.get(&key()).await.unwrap().as_deref(),
            Some(&value[..])
        );
    }

    #[tokio::test]
    async fn commands_share_one_connection() {
        let (address, accepted) = fake_server(Behavior::Serve).await;
        let cache = StellerCache::new(address).unwrap();
        for _ in 0..5 {
            cache.ping().await.unwrap();
        }
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reconnects_after_the_server_closes_the_connection() {
        let (address, accepted) = fake_server(Behavior::CloseAfterOne).await;
        let cache = StellerCache::new(address).unwrap();
        cache
            .set(&key(), b"v", Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(cache.get(&key()).await.unwrap().as_deref(), Some(&b"v"[..]));
        assert!(cache.ping().await.is_ok());
        assert_eq!(accepted.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_server_that_never_replies_costs_one_deadline() {
        let (address, _) = fake_server(Behavior::Silent).await;
        let cache = StellerCache::new(address).unwrap();
        let started = Instant::now();
        let error = cache.ping().await.unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < DEADLINE * 3, "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn nothing_listening_is_an_error_not_a_hang() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let cache = StellerCache::new(address).unwrap();
        assert!(cache.get(&key()).await.is_err());
    }

    #[tokio::test]
    async fn an_error_reply_is_an_error_and_closes_the_connection() {
        let (address, accepted) = fake_server(Behavior::Refuse).await;
        let cache = StellerCache::new(address).unwrap();
        for _ in 0..3 {
            let error = cache.ping().await.unwrap_err();
            assert!(error.to_string().contains("ERR refused"), "{error}");
        }
        assert_eq!(accepted.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn replies_left_over_from_a_rejected_command_are_never_read() {
        let (address, _) = fake_server(Behavior::RejectThenFlood).await;
        let cache = StellerCache::new(address).unwrap();
        assert!(
            cache
                .set(&key(), b"v", Duration::from_mins(1))
                .await
                .is_err()
        );
        assert_eq!(cache.get(&key()).await.unwrap(), None);
        assert!(cache.ping().await.is_ok());
    }
}
