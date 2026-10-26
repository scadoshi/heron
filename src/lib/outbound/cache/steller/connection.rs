//! One TCP connection to the server, and a request and reply over it.

use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

use crate::outbound::cache::steller::resp::{self, Reply, RespError};

/// Why a command got no reply.
#[derive(Debug, Error)]
pub enum ConnectionError {
    /// Connecting, writing, or reading failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The server closed the connection.
    #[error("connection closed by the server")]
    Closed,
    /// The deadline passed.
    #[error("timed out after {0:?}")]
    TimedOut(Duration),
    /// The reply did not follow the protocol.
    #[error(transparent)]
    Protocol(#[from] RespError),
}

/// An open connection and the bytes read from it that no reply has used yet.
#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    unread: Vec<u8>,
}

impl Connection {
    /// Connects to `address`, giving up after `deadline`.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::TimedOut`] when `deadline` passes first, and
    /// [`ConnectionError::Io`] when the connection is refused or `TCP_NODELAY`
    /// cannot be set.
    pub async fn open(address: SocketAddr, deadline: Duration) -> Result<Self, ConnectionError> {
        let stream = time::timeout(deadline, TcpStream::connect(address))
            .await
            .map_err(|_| ConnectionError::TimedOut(deadline))??;
        // Commands are small and each waits for its reply, so there is nothing for
        // Nagle's algorithm to batch.
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            unread: Vec::new(),
        })
    }

    /// Sends `command` and reads one reply, giving up after `deadline`.
    ///
    /// After any error the connection is in an unknown state and must be dropped.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::TimedOut`] when `deadline` passes before a whole reply,
    /// [`ConnectionError::Io`] when writing or reading fails,
    /// [`ConnectionError::Closed`] when the server closes the connection first, and
    /// [`ConnectionError::Protocol`] when the reply breaks RESP.
    pub async fn roundtrip(
        &mut self,
        command: &[u8],
        deadline: Duration,
    ) -> Result<Reply, ConnectionError> {
        time::timeout(deadline, self.exchange(command))
            .await
            .map_err(|_| ConnectionError::TimedOut(deadline))?
    }

    async fn exchange(&mut self, command: &[u8]) -> Result<Reply, ConnectionError> {
        self.stream.write_all(command).await?;
        loop {
            if let Some((reply, used)) = resp::parse(&self.unread)? {
                self.unread.drain(..used);
                return Ok(reply);
            }
            if self.stream.read_buf(&mut self.unread).await? == 0 {
                return Err(ConnectionError::Closed);
            }
        }
    }
}
