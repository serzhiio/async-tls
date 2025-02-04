use crate::common::tls_state::TlsState;

use crate::client;

use futures::io::{AsyncRead, AsyncWrite};
use rustls::{ClientConfig, ClientSession};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use webpki::DNSNameRef;

/// The TLS connecting part. The acceptor drives
/// the client side of the TLS handshake process. It works
/// on any asynchronous stream.
///
/// It provides a simple interface (`connect`), returning a future
/// that will resolve when the handshake process completed. On
/// success, it will hand you an async `TlsStream`.
///
/// To create a `TlsConnector` with a non-default configuation, create
/// a `rusttls::ClientConfig` and call `.into()` on it.
///
/// ## Example
///
/// ```rust
/// #![feature(async_await)]
///
/// use async_tls::TlsConnector;
///
/// async_std::task::block_on(async {
///     let connector = TlsConnector::default();
///     let tcp_stream = async_std::net::TcpStream::connect("example.com").await?;
///     let handshake = connector.connect("example.com", tcp_stream)?;
///     let encrypted_stream = handshake.await?;
///
///     Ok(()) as async_std::io::Result<()>
/// });
/// ```
#[derive(Clone)]
pub struct TlsConnector {
    inner: Arc<ClientConfig>,
    #[cfg(feature = "early-data")]
    early_data: bool,
}

impl From<Arc<ClientConfig>> for TlsConnector {
    fn from(inner: Arc<ClientConfig>) -> TlsConnector {
        TlsConnector {
            inner,
            #[cfg(feature = "early-data")]
            early_data: false,
        }
    }
}

impl Default for TlsConnector {
    fn default() -> Self {
        let mut config = ClientConfig::new();
        config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
        Arc::new(config).into()
    }
}

impl TlsConnector {
    /// Create a new TlsConnector with default configuration.
    ///
    /// This is the same as calling `TlsConnector::default()`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Enable 0-RTT.
    ///
    /// You must also set `enable_early_data` to `true` in `ClientConfig`.
    #[cfg(feature = "early-data")]
    pub fn early_data(mut self, flag: bool) -> TlsConnector {
        self.early_data = flag;
        self
    }

    /// Connect to a server. `stream` can be any type implementing `AsyncRead` and `AsyncWrite`,
    /// such as TcpStreams or Unix domain sockets.
    ///
    /// The function will return an error if the domain is not of valid format.
    /// Otherwise, it will return a `Connect` Future, representing the connecting part of a
    /// Tls handshake. It will resolve when the handshake is over.
    #[inline]
    pub fn connect<'a, IO>(&self, domain: impl AsRef<str>, stream: IO) -> io::Result<Connect<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
    {
        self.connect_with(domain, stream, |_| ())
    }

    // NOTE: Currently private, exposing ClientSession exposes rusttls
    // Early data should be exposed differently
    fn connect_with<'a, IO, F>(
        &self,
        domain: impl AsRef<str>,
        stream: IO,
        f: F,
    ) -> io::Result<Connect<IO>>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
        F: FnOnce(&mut ClientSession),
    {
        let domain = DNSNameRef::try_from_ascii_str(domain.as_ref())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid domain"))?;
        let mut session = ClientSession::new(&self.inner, domain);
        f(&mut session);

        #[cfg(not(feature = "early-data"))]
        {
            Ok(Connect(client::MidHandshake::Handshaking(
                client::TlsStream {
                    session,
                    io: stream,
                    state: TlsState::Stream,
                },
            )))
        }

        #[cfg(feature = "early-data")]
        {
            Ok(Connect(if self.early_data {
                client::MidHandshake::EarlyData(client::TlsStream {
                    session,
                    io: stream,
                    state: TlsState::EarlyData,
                    early_data: (0, Vec::new()),
                })
            } else {
                client::MidHandshake::Handshaking(client::TlsStream {
                    session,
                    io: stream,
                    state: TlsState::Stream,
                    early_data: (0, Vec::new()),
                })
            }))
        }
    }
}

/// Future returned from `TlsConnector::connect` which will resolve
/// once the connection handshake has finished.
pub struct Connect<IO>(client::MidHandshake<IO>);

impl<IO: AsyncRead + AsyncWrite + Unpin> Future for Connect<IO> {
    type Output = io::Result<client::TlsStream<IO>>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}
