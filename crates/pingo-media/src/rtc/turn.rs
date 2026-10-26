//! TURN I/O stays on bounded workers so permission transactions and TCP backpressure cannot stall DSP.
use super::{Result, rtc_error};
use async_trait::async_trait;
use futures_util::{StreamExt, stream::FuturesUnordered};
use pingo_proto::ice::{IceServer, TurnTransport, TurnUrl};
use std::{collections::HashMap, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf},
    net::{TcpStream, UdpSocket},
    sync::{Mutex, mpsc},
    task::JoinSet,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{self, pki_types::ServerName},
};
use webrtc_util::Conn;

pub(super) struct Datagram {
    pub source: SocketAddr,
    pub destination: SocketAddr,
    pub data: Vec<u8>,
}
struct Outgoing {
    destination: SocketAddr,
    data: Vec<u8>,
}

pub(super) struct Relays {
    senders: HashMap<SocketAddr, mpsc::Sender<Outgoing>>,
    pub candidates: Vec<(SocketAddr, SocketAddr)>,
    pub incoming: mpsc::Receiver<Datagram>,
    // Keep recv pending when no allocations exist, instead of polling a closed channel.
    _incoming_sender: mpsc::Sender<Datagram>,
    // Dropping JoinSet aborts the workers, whose Allocation guards release sockets and timers.
    _workers: JoinSet<()>,
}
impl Relays {
    pub async fn gather(servers: &[IceServer]) -> Result<Self> {
        if servers.len() > 8 || servers.iter().map(|s| s.urls.len()).sum::<usize>() > 8 {
            return Err(super::Error::Invalid("too many TURN URLs"));
        }
        let mut pending = FuturesUnordered::new();
        for server in servers {
            if server.username.len() > 1024 || server.credential.len() > 1024 {
                return Err(super::Error::Invalid("TURN credentials exceed limits"));
            }
            for url in &server.urls {
                let url = TurnUrl::parse(url).map_err(rtc_error)?;
                pending.push(async move {
                    tokio::time::timeout(Duration::from_secs(4), allocate(&url, server)).await
                });
            }
        }
        let (incoming_tx, incoming) = mpsc::channel(256);
        let mut result = Self {
            senders: HashMap::new(),
            candidates: vec![],
            incoming,
            _incoming_sender: incoming_tx.clone(),
            _workers: JoinSet::new(),
        };
        while let Some(allocation) = pending.next().await {
            let allocation = match allocation {
                Ok(Ok(a)) => a,
                // Library errors can contain credential-bearing STUN messages; never log them.
                _ => {
                    tracing::warn!("TURN allocation unavailable; trying other ICE candidates");
                    continue;
                }
            };
            let relay = allocation.relay.as_ref().expect("allocated").clone();
            let address = relay.local_addr().map_err(rtc_error)?;
            let base = allocation.base.local_addr().map_err(rtc_error)?;
            result.candidates.push((address, base));
            let (tx, mut rx) = mpsc::channel::<Outgoing>(256);
            result.senders.insert(address, tx);
            let incoming = incoming_tx.clone();
            result._workers.spawn(async move {
                let _allocation = allocation;
                let mut io_workers = JoinSet::new();
                let writer = relay.clone();
                io_workers.spawn(async move {
                    while let Some(packet) = rx.recv().await {
                        if writer
                            .send_to(&packet.data, packet.destination)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                io_workers.spawn(async move {
                    let mut buf = vec![0; 65_535];
                    while let Ok((len, source)) = relay.recv_from(&mut buf).await {
                        let _ = incoming.try_send(Datagram {
                            source,
                            destination: address,
                            data: buf[..len].to_vec(),
                        });
                    }
                });
                io_workers.join_next().await;
                tracing::debug!("TURN transport closed");
            });
        }
        Ok(result)
    }
    pub fn send(&self, source: SocketAddr, destination: SocketAddr, data: &[u8]) -> bool {
        let Some(sender) = self.senders.get(&source) else {
            return false;
        };
        let _ = sender.try_send(Outgoing {
            destination,
            data: data.to_vec(),
        });
        true
    }
}

struct Allocation {
    client: Arc<turn::client::Client>,
    base: Arc<dyn Conn + Send + Sync>,
    relay: Option<Arc<dyn Conn + Send + Sync>>,
}
impl Drop for Allocation {
    fn drop(&mut self) {
        let (client, base, relay) = (self.client.clone(), self.base.clone(), self.relay.clone());
        tokio::spawn(async move {
            // A dead relay must not prevent local resources from being released.
            if let Some(relay) = relay {
                let _ = tokio::time::timeout(Duration::from_secs(1), relay.close()).await;
            }
            let _ = client.close().await;
            let _ = base.close().await;
        });
    }
}

async fn allocate(url: &TurnUrl, credentials: &IceServer) -> Result<Allocation> {
    let address = tokio::net::lookup_host((url.host.as_str(), url.port))
        .await?
        .next()
        .ok_or(super::Error::Invalid("TURN hostname has no addresses"))?;
    let base: Arc<dyn Conn + Send + Sync> = match url.transport {
        TurnTransport::Udp => {
            let socket = UdpSocket::bind(if address.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            })
            .await?;
            Arc::new(socket)
        }
        TurnTransport::Tcp | TurnTransport::Tls => {
            let socket = TcpStream::connect(address).await?;
            socket.set_nodelay(true)?;
            let local = socket.local_addr()?;
            if url.transport == TurnTransport::Tls {
                let mut roots = rustls::RootCertStore::from_iter(
                    webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                );
                // Respect OS trust (including private self-hosted CAs), alongside public WebPKI roots.
                for cert in rustls_native_certs::load_native_certs().certs {
                    let _ = roots.add(cert);
                }
                let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .map_err(rtc_error)?
                .with_root_certificates(roots)
                .with_no_client_auth();
                let name = ServerName::try_from(url.host.clone()).map_err(rtc_error)?;
                let stream = TlsConnector::from(Arc::new(tls))
                    .connect(name, socket)
                    .await?;
                Arc::new(StreamConn::new(stream, local, address))
            } else {
                Arc::new(StreamConn::new(socket, local, address))
            }
        }
    };
    let client = Arc::new(
        turn::client::Client::new(turn::client::ClientConfig {
            stun_serv_addr: String::new(),
            turn_serv_addr: address.to_string(),
            username: credentials.username.clone(),
            password: credentials.credential.clone(),
            realm: String::new(),
            software: "pingo".into(),
            rto_in_ms: 200,
            conn: base.clone(),
            vnet: None,
        })
        .await
        .map_err(rtc_error)?,
    );
    let mut allocation = Allocation {
        client,
        base,
        relay: None,
    };
    allocation.client.listen().await.map_err(rtc_error)?;
    allocation.relay = Some(Arc::new(
        allocation.client.allocate().await.map_err(rtc_error)?,
    ));
    Ok(allocation)
}

/// TURN over streams uses STUN lengths and padded ChannelData frames, not a two-byte RTP length prefix.
struct StreamConn<S> {
    reader: Mutex<ReadHalf<S>>,
    writer: Mutex<WriteHalf<S>>,
    local: SocketAddr,
    remote: SocketAddr,
}
impl<S: AsyncRead + AsyncWrite> StreamConn<S> {
    fn new(stream: S, local: SocketAddr, remote: SocketAddr) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
            local,
            remote,
        }
    }
}
#[async_trait]
impl<S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> Conn for StreamConn<S> {
    async fn connect(&self, _: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }
    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        self.recv_from(buf).await.map(|(n, _)| n)
    }
    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let mut reader = self.reader.lock().await;
        let mut header = [0; 4];
        reader.read_exact(&mut header).await?;
        let body = u16::from_be_bytes([header[2], header[3]]) as usize;
        let channel = header[0] & 0xc0 == 0x40;
        if !channel && header[0] & 0xc0 != 0 {
            return Err(io::Error::other("invalid TURN stream frame").into());
        }
        let len = if channel { 4 + body } else { 20 + body };
        let padded = if channel { (len + 3) & !3 } else { len };
        if padded > buf.len() {
            return Err(io::Error::other("TURN frame exceeds buffer").into());
        }
        buf[..4].copy_from_slice(&header);
        reader.read_exact(&mut buf[4..padded]).await?;
        Ok((len, self.remote))
    }
    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.send_to(buf, self.remote).await
    }
    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        if target != self.remote {
            return Err(io::Error::other("TURN stream destination mismatch").into());
        }
        let mut writer = self.writer.lock().await;
        writer.write_all(buf).await?;
        if buf.first().is_some_and(|b| b & 0xc0 == 0x40) {
            let padding = (4 - buf.len() % 4) % 4;
            writer.write_all(&[0; 3][..padding]).await?;
        }
        Ok(buf.len())
    }
    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local)
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote)
    }
    async fn close(&self) -> webrtc_util::Result<()> {
        self.writer.lock().await.shutdown().await?;
        Ok(())
    }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires the restricted local coturn; see deploy/test-turn.sh"]
    async fn coturn_restricts_peer_destinations() {
        let url = TurnUrl::parse(&std::env::var("PINGO_TEST_TURN_URL").unwrap()).unwrap();
        let secret = std::env::var("PINGO_TEST_TURN_SECRET").unwrap();
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let username = format!("{expires}:acl-test");
        let credential = pingo_proto::ice::turn_password(&username, secret.as_bytes());
        let server = IceServer {
            urls: vec![],
            username,
            credential,
        };
        let allocation = tokio::time::timeout(Duration::from_secs(4), allocate(&url, &server))
            .await
            .unwrap()
            .unwrap();
        // This documentation address is outside PINGO_TURN_PEER_IPS; coturn must reject permission.
        let sent = tokio::time::timeout(
            Duration::from_secs(4),
            allocation
                .relay
                .as_ref()
                .unwrap()
                .send_to(b"denied", "198.51.100.1:9".parse().unwrap()),
        )
        .await
        .unwrap();
        assert!(
            sent.is_err(),
            "a TURN allocation must not permit arbitrary relay destinations"
        );
    }
    #[tokio::test]
    async fn stream_framing_survives_fragmentation_and_channel_padding() {
        let (a, b) = tokio::io::duplex(1024);
        let addr = "127.0.0.1:3478".parse().unwrap();
        let conn = StreamConn::new(a, addr, addr);
        let mut peer = b;
        let writer = tokio::spawn(async move {
            // An odd-sized ChannelData payload followed by a STUN message.
            for bytes in [
                &[0x40, 1][..],
                &[0, 3, 9][..],
                &[8, 7, 0][..],
                &[0, 1, 0, 0][..],
                &[0; 16][..],
            ] {
                peer.write_all(bytes).await.unwrap();
                tokio::task::yield_now().await;
            }
            let mut frame = [0; 8];
            peer.read_exact(&mut frame).await.unwrap();
            frame
        });
        let mut buf = [0; 64];
        assert_eq!(conn.recv(&mut buf).await.unwrap(), 7);
        assert_eq!(&buf[..7], &[0x40, 1, 0, 3, 9, 8, 7]);
        assert_eq!(conn.recv(&mut buf).await.unwrap(), 20);
        conn.send(&[0x40, 1, 0, 1, 42]).await.unwrap();
        assert_eq!(writer.await.unwrap(), [0x40, 1, 0, 1, 42, 0, 0, 0]);
    }
}
