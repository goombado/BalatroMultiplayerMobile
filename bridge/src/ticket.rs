//! Auth session ticket layout. Pure functions only, so the byte format is unit-tested apart from
//! the network.
//!
//! Ported from node-steam-user (`components/appauth.js`, MIT) and steam-appticket
//! (`parseAppTicket.js`, MIT). See NOTICE. The ticket is three length-prefixed parts:
//!
//! ```text
//! u32 gc_len   | GC token (20 bytes, from ClientGameConnectTokens)
//! u32 24       | session header: 1, 2, public IPv4, 0, ms since logon, connection count
//! u32 own_len  | app ownership ticket (from ClientGetAppOwnershipTicket)
//! ```
//!
//! All integers are little-endian. Steam registers the first two parts (the "auth portion") through
//! ClientAuthList, keyed by its CRC32.

pub const SESSION_HEADER_LEN: u32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionHeader {
    /// Public IPv4 from ClientLogOnResponse, as the big-endian number of a.b.c.d (`u32::from(Ipv4Addr)`).
    pub public_ip: u32,
    pub ms_since_logon: u32,
    pub connection_count: u32,
}

pub fn build_session_ticket(gc_token: &[u8], header: &SessionHeader, ownership_ticket: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + gc_token.len() + 4 + SESSION_HEADER_LEN as usize + 4 + ownership_ticket.len());
    out.extend_from_slice(&(gc_token.len() as u32).to_le_bytes());
    out.extend_from_slice(gc_token);
    out.extend_from_slice(&SESSION_HEADER_LEN.to_le_bytes());
    for v in [1, 2, header.public_ip, 0, header.ms_since_logon, header.connection_count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(ownership_ticket.len() as u32).to_le_bytes());
    out.extend_from_slice(ownership_ticket);
    out
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn read_u64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

/// The GC token and session header (52 bytes for a 20-byte token): the part ClientAuthList carries.
pub fn auth_portion(ticket: &[u8]) -> Option<&[u8]> {
    let gc_len = read_u32(ticket, 0)? as usize;
    let header_len = read_u32(ticket, 4 + gc_len)? as usize;
    if header_len != SESSION_HEADER_LEN as usize {
        return None;
    }
    ticket.get(..4 + gc_len + 4 + header_len)
}

pub fn ticket_crc(ticket: &[u8]) -> Option<u32> {
    auth_portion(ticket).map(crc32fast::hash)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipInfo {
    pub steam_id: u64,
    pub app_id: u32,
    /// Same encoding as `SessionHeader::public_ip`.
    pub external_ip: u32,
    pub generated: u32,
    pub expires: u32,
}

/// Reads the fixed fields at the front of an app ownership ticket:
/// `u32 len, u32 version, u64 steamid, u32 appid, u32 ext ip, u32 int ip, u32 flags, u32 generated, u32 expires`.
/// The IPs are stored as little-endian integers of the big-endian address.
pub fn parse_ownership(ticket: &[u8]) -> Option<OwnershipInfo> {
    let len = read_u32(ticket, 0)? as usize;
    if len < 40 || len > ticket.len() {
        return None;
    }
    Some(OwnershipInfo {
        steam_id: read_u64(ticket, 8)?,
        app_id: read_u32(ticket, 16)?,
        external_ip: read_u32(ticket, 20)?,
        generated: read_u32(ticket, 32)?,
        expires: read_u32(ticket, 36)?,
    })
}

/// Same rule as node-steam-user: reuse a cached ownership ticket only while it has more than six
/// hours left and was issued for our current public IP.
pub fn ownership_reusable(info: &OwnershipInfo, now_unix: u64, public_ip: u32) -> bool {
    (info.expires as u64) > now_unix + 6 * 3600 && info.external_ip == public_ip
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fake ownership ticket with the real field layout (signature replaced by zeros).
    fn fake_ownership(steam_id: u64, app_id: u32, ip: u32, expires: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // length, patched below
        body.extend_from_slice(&4u32.to_le_bytes()); // version
        body.extend_from_slice(&steam_id.to_le_bytes());
        body.extend_from_slice(&app_id.to_le_bytes());
        body.extend_from_slice(&ip.to_le_bytes());
        body.extend_from_slice(&0x0a00_0002u32.to_le_bytes()); // internal ip
        body.extend_from_slice(&0u32.to_le_bytes()); // flags
        body.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        body.extend_from_slice(&expires.to_le_bytes());
        body.extend_from_slice(&[0u8; 10]); // licenses / dlc / reserved
        let len = body.len() as u32;
        body[0..4].copy_from_slice(&len.to_le_bytes());
        body.extend_from_slice(&[0u8; 128]); // signature
        body
    }

    #[test]
    fn layout_matches_node_steam_user() {
        let gc = [0xabu8; 20];
        let own = fake_ownership(76561198000000000, 2379780, 0x0102_0304, 1_800_000_000);
        let h = SessionHeader { public_ip: 0x0102_0304, ms_since_logon: 5000, connection_count: 1 };
        let t = build_session_ticket(&gc, &h, &own);

        assert_eq!(t.len(), 4 + 20 + 4 + 24 + 4 + own.len());
        assert_eq!(&t[0..4], &20u32.to_le_bytes());
        assert_eq!(&t[4..24], &gc);
        assert_eq!(&t[24..28], &24u32.to_le_bytes());
        assert_eq!(&t[28..32], &1u32.to_le_bytes());
        assert_eq!(&t[32..36], &2u32.to_le_bytes());
        // IPv4 1.2.3.4 -> 0x01020304 written little-endian, as StdLib.IPv4.stringToInt + writeUint32 does.
        assert_eq!(&t[36..40], &[4, 3, 2, 1]);
        assert_eq!(&t[40..44], &0u32.to_le_bytes());
        assert_eq!(&t[44..48], &5000u32.to_le_bytes());
        assert_eq!(&t[48..52], &1u32.to_le_bytes());
        assert_eq!(&t[52..56], &(own.len() as u32).to_le_bytes());
        assert_eq!(&t[56..], &own[..]);
    }

    #[test]
    fn spike_ticket_length() {
        // Spike 0.1: real tickets were 234 bytes, i.e. a 178-byte ownership ticket (50 + 128 signature).
        let own = vec![0u8; 178];
        let t = build_session_ticket(&[0u8; 20], &SessionHeader { public_ip: 0, ms_since_logon: 0, connection_count: 1 }, &own);
        assert_eq!(t.len(), 234);
    }

    #[test]
    fn auth_portion_is_52_bytes_and_crc_covers_it() {
        let own = fake_ownership(1, 2379780, 0, 0);
        let t = build_session_ticket(&[7u8; 20], &SessionHeader { public_ip: 9, ms_since_logon: 1, connection_count: 2 }, &own);
        let a = auth_portion(&t).unwrap();
        assert_eq!(a.len(), 52);
        assert_eq!(a, &t[..52]);
        assert_eq!(ticket_crc(&t), Some(crc32fast::hash(&t[..52])));
    }

    #[test]
    fn auth_portion_rejects_garbage() {
        assert_eq!(auth_portion(&[]), None);
        assert_eq!(auth_portion(&[20, 0, 0, 0, 1, 2]), None);
        let mut t = build_session_ticket(&[0u8; 20], &SessionHeader { public_ip: 0, ms_since_logon: 0, connection_count: 0 }, &[]);
        t[24] = 23; // wrong session header length
        assert_eq!(auth_portion(&t), None);
    }

    #[test]
    fn ownership_fields_and_reuse_rule() {
        let own = fake_ownership(76561198000000001, 2379780, 0x0a0b_0c0d, 1_800_000_000);
        let info = parse_ownership(&own).unwrap();
        assert_eq!(info.steam_id, 76561198000000001);
        assert_eq!(info.app_id, 2379780);
        assert_eq!(info.external_ip, 0x0a0b_0c0d);
        assert_eq!(info.expires, 1_800_000_000);

        assert!(ownership_reusable(&info, 1_700_000_000, 0x0a0b_0c0d));
        assert!(!ownership_reusable(&info, 1_800_000_000 - 3600, 0x0a0b_0c0d), "under six hours left");
        assert!(!ownership_reusable(&info, 1_700_000_000, 0x0102_0304), "public IP changed");
        assert_eq!(parse_ownership(&[1, 2, 3]), None);
    }
}
