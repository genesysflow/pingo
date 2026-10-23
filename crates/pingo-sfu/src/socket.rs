//! str0m needs the concrete ICE destination; a wildcard socket's local_addr is not one.
use std::io;
use std::net::{IpAddr, SocketAddr};
use tokio::net::UdpSocket;

#[cfg(target_os = "linux")]
pub(crate) fn enable_destination(socket: &UdpSocket) -> io::Result<()> {
    use nix::sys::socket::{setsockopt, sockopt};
    if socket.local_addr()?.is_ipv4() {
        setsockopt(socket, sockopt::Ipv4PacketInfo, &true)?;
    } else {
        setsockopt(socket, sockopt::Ipv6RecvPacketInfo, &true)?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn enable_destination(_: &UdpSocket) -> io::Result<()> {
    Ok(())
}

fn candidate_destination(local: SocketAddr, advertised: &[IpAddr]) -> io::Result<SocketAddr> {
    // A sole public address may be NAT-mapped to the actual ingress interface.
    let ip = if advertised.contains(&local.ip()) {
        local.ip()
    } else if let [ip] = advertised {
        *ip
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UDP destination has no matching ICE candidate",
        ));
    };
    Ok(SocketAddr::new(ip, local.port()))
}

#[cfg(target_os = "linux")]
pub(crate) async fn receive(
    socket: &UdpSocket,
    buffer: &mut [u8],
    advertised: &[IpAddr],
) -> io::Result<(usize, SocketAddr, SocketAddr)> {
    use nix::sys::socket::{ControlMessageOwned, MsgFlags, SockaddrStorage, recvmsg};
    use std::io::IoSliceMut;
    use std::os::fd::AsRawFd;
    loop {
        socket.readable().await?;
        let result = socket.try_io(tokio::io::Interest::READABLE, || {
            let mut iov = [IoSliceMut::new(buffer)];
            let mut control = nix::cmsg_space!(nix::libc::in_pktinfo, nix::libc::in6_pktinfo);
            let message = recvmsg::<SockaddrStorage>(
                socket.as_raw_fd(),
                &mut iov,
                Some(&mut control),
                MsgFlags::MSG_DONTWAIT,
            )?;
            let address = message
                .address
                .ok_or_else(|| io::Error::other("missing UDP source"))?;
            let source = if let Some(v4) = address.as_sockaddr_in() {
                SocketAddr::new(v4.ip().into(), v4.port())
            } else if let Some(v6) = address.as_sockaddr_in6() {
                SocketAddr::V6(std::net::SocketAddrV6::new(
                    v6.ip(),
                    v6.port(),
                    v6.flowinfo(),
                    v6.scope_id(),
                ))
            } else {
                return Err(io::Error::other("unsupported UDP source family"));
            };
            let mut destination = socket.local_addr()?;
            for control in message.cmsgs()? {
                match control {
                    ControlMessageOwned::Ipv4PacketInfo(info) => {
                        destination.set_ip(IpAddr::V4(info.ipi_addr.s_addr.to_ne_bytes().into()))
                    }
                    ControlMessageOwned::Ipv6PacketInfo(info) => {
                        destination.set_ip(IpAddr::V6(info.ipi6_addr.s6_addr.into()))
                    }
                    _ => {}
                }
            }
            Ok((
                message.bytes,
                source,
                candidate_destination(destination, advertised)?,
            ))
        });
        match result {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            other => return other,
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn receive(
    socket: &UdpSocket,
    buffer: &mut [u8],
    advertised: &[IpAddr],
) -> io::Result<(usize, SocketAddr, SocketAddr)> {
    let (size, source) = socket.recv_from(buffer).await?;
    Ok((
        size,
        source,
        candidate_destination(socket.local_addr()?, advertised)?,
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wildcard_receive_preserves_each_ingress_destination() {
        let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        enable_destination(&socket).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let advertised = ["127.0.0.1".parse().unwrap(), "127.0.0.2".parse().unwrap()];
        let port = socket.local_addr().unwrap().port();
        let mut bytes = [0; 8];
        for ip in advertised.into_iter().rev() {
            let destination = SocketAddr::new(ip, port);
            sender.send_to(b"packet", destination).await.unwrap();
            let (size, source, received_at) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                receive(&socket, &mut bytes, &advertised),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&bytes[..size], b"packet");
            assert_eq!(source, sender.local_addr().unwrap());
            assert_eq!(received_at, destination);
        }
    }

    #[test]
    fn sole_public_candidate_maps_the_private_ingress_interface() {
        assert_eq!(
            candidate_destination(
                "10.0.0.2:7882".parse().unwrap(),
                &["203.0.113.1".parse().unwrap()]
            )
            .unwrap(),
            "203.0.113.1:7882".parse().unwrap(),
        );
    }
}
