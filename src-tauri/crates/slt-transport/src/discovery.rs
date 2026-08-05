//! Vehicle discovery over UDP broadcast.
//!
//! Both transports announce themselves on a UDP port, so the user never has to
//! know the gateway's IP address. HSFZ uses port 6811 with a control word 0x11
//! identification frame; DoIP uses port 13400 with an ISO 13400 vehicle
//! identification request.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::doip::{self, VehicleAnnouncement};
use crate::error::Result;
use crate::hsfz;
use crate::Protocol;

/// A vehicle gateway found on the network.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiscoveredVehicle {
    pub ip: IpAddr,
    pub port: u16,
    pub protocol: Protocol,
    /// VIN, when the announcement included one.
    pub vin: Option<String>,
    /// Gateway logical address, when reported.
    pub logical_address: Option<u16>,
}

/// Broadcasts identification requests on both protocols and collects replies
/// until `timeout` elapses.
///
/// Both are probed because the chassis generation is not known in advance, and
/// a G-series car answers DoIP while an F-series answers HSFZ. Running them
/// concurrently keeps discovery fast.
pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredVehicle>> {
    let (doip_result, hsfz_result) =
        tokio::join!(discover_doip(timeout), discover_hsfz(timeout));

    let mut found = Vec::new();
    match doip_result {
        Ok(mut v) => found.append(&mut v),
        Err(e) => tracing::warn!(error = %e, "DoIP discovery failed"),
    }
    match hsfz_result {
        Ok(mut v) => found.append(&mut v),
        Err(e) => tracing::warn!(error = %e, "HSFZ discovery failed"),
    }

    // A gateway that answers both protocols would otherwise appear twice.
    found.dedup_by(|a, b| a.ip == b.ip && a.protocol == b.protocol);
    Ok(found)
}

/// Sends an ISO 13400 vehicle identification request to the broadcast address.
pub async fn discover_doip(timeout: Duration) -> Result<Vec<DiscoveredVehicle>> {
    let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).await?;
    socket.set_broadcast(true)?;

    let request = doip::Message::vehicle_ident_request().encode();
    socket
        .send_to(
            &request,
            SocketAddr::from((Ipv4Addr::BROADCAST, doip::PORT)),
        )
        .await?;

    let mut found = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = [0u8; 1024];

    while let Ok(Ok((len, peer))) =
        tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await
    {
        let Ok(Some((message, _))) = doip::decode_message(&buf[..len]) else {
            continue;
        };
        if message.payload_type != doip::PayloadType::VehicleAnnouncement {
            continue;
        }
        let announcement = VehicleAnnouncement::parse(&message.payload).ok();
        found.push(DiscoveredVehicle {
            ip: peer.ip(),
            port: doip::PORT,
            protocol: Protocol::DoIp,
            vin: announcement.as_ref().map(|a| a.vin.clone()),
            logical_address: announcement.as_ref().map(|a| a.logical_address),
        });
    }

    Ok(found)
}

/// Sends an HSFZ identification request to the broadcast address.
pub async fn discover_hsfz(timeout: Duration) -> Result<Vec<DiscoveredVehicle>> {
    let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).await?;
    socket.set_broadcast(true)?;

    // A zero-length identification frame asks the gateway to identify itself.
    let request = hsfz::Frame {
        control: hsfz::ControlWord::VehicleIdent,
        addresses: None,
        payload: Vec::new(),
    }
    .encode();
    socket
        .send_to(
            &request,
            SocketAddr::from((Ipv4Addr::BROADCAST, hsfz::CONTROL_PORT)),
        )
        .await?;

    let mut found = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = [0u8; 1024];

    while let Ok(Ok((len, peer))) =
        tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await
    {
        let Ok(Some((frame, _))) = hsfz::decode_frame(&buf[..len]) else {
            continue;
        };
        if frame.control != hsfz::ControlWord::VehicleIdent {
            continue;
        }
        // The identification payload begins with the VIN as ASCII. Length
        // varies by gateway firmware, so take the leading printable run.
        let vin = extract_vin(&frame.payload);
        found.push(DiscoveredVehicle {
            ip: peer.ip(),
            port: hsfz::DIAGNOSTIC_PORT,
            protocol: Protocol::Hsfz,
            vin,
            logical_address: None,
        });
    }

    Ok(found)
}

/// Pulls a VIN out of an HSFZ identification payload.
///
/// Gateway firmware varies in what it appends after the VIN, so rather than
/// assuming a fixed layout this takes the leading run of VIN-legal characters
/// and only accepts it at the standard 17-character length.
fn extract_vin(payload: &[u8]) -> Option<String> {
    let text: String = payload
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric())
        .map(|&b| b as char)
        .collect();
    (text.len() == 17).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_full_length_vin() {
        assert_eq!(
            extract_vin(b"WBA8E9G51GNT12345"),
            Some("WBA8E9G51GNT12345".to_string())
        );
    }

    #[test]
    fn stops_at_a_non_alphanumeric_byte() {
        assert_eq!(
            extract_vin(b"WBA8E9G51GNT12345\0\xde\xad"),
            Some("WBA8E9G51GNT12345".to_string())
        );
    }

    #[test]
    fn rejects_a_wrong_length_run() {
        assert_eq!(extract_vin(b"SHORT"), None);
        assert_eq!(extract_vin(b""), None);
        assert_eq!(extract_vin(b"WAYTOOLONGFORAVINNUMBER12345"), None);
    }
}
