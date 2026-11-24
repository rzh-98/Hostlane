//! Just enough DER to report on a server certificate: subject and issuer names,
//! subject alternative names and the validity period. Signatures are not checked.

use std::net::{Ipv4Addr, Ipv6Addr};

use hostlane_protocol::TlsInfo;

const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const OID_ORGANIZATION: &[u8] = &[0x55, 0x04, 0x0a];
const OID_SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];
const DAY_MS: i64 = 86_400_000;

/// One DER element: (tag, content, rest).
fn der(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = data.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let length = rest[..count]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | *b as usize);
        (length, &rest[count..])
    };
    if rest.len() < length {
        return None;
    }
    Some((tag, &rest[..length], &rest[length..]))
}

fn items(mut data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    while let Some((tag, content, rest)) = der(data) {
        out.push((tag, content));
        data = rest;
    }
    out
}

fn name_attribute(name: &[u8], oid: &[u8]) -> Option<String> {
    items(name).into_iter().find_map(|(_, rdn)| {
        items(rdn).into_iter().find_map(|(_, pair)| {
            let pair = items(pair);
            let (_, id) = pair.first()?;
            let (_, value) = pair.get(1)?;
            (*id == oid).then(|| String::from_utf8_lossy(value).into_owned())
        })
    })
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// UTCTime (`YYMMDDHHMMSSZ`) or GeneralizedTime (`YYYYMMDDHHMMSSZ`) as Unix ms.
fn time(tag: u8, content: &[u8]) -> Option<u64> {
    let digits = std::str::from_utf8(content).ok()?.strip_suffix('Z')?;
    let (year, rest) = match tag {
        0x17 => {
            let year: i64 = digits.get(..2)?.parse().ok()?;
            (
                if year >= 50 { 1900 + year } else { 2000 + year },
                &digits[2..],
            )
        }
        0x18 => (digits.get(..4)?.parse().ok()?, &digits[4..]),
        _ => return None,
    };
    if rest.len() != 10 {
        return None;
    }
    let field = |at: usize| -> Option<i64> { rest.get(at..at + 2)?.parse().ok() };
    let days = days_from_civil(year, field(0)?, field(2)?);
    let secs = days * 86_400 + field(4)? * 3600 + field(6)? * 60 + field(8)?;
    u64::try_from(secs).ok().map(|secs| secs * 1000)
}

fn alt_names(extensions: &[u8]) -> Vec<String> {
    let Some((_, list)) = items(extensions).into_iter().next() else {
        return Vec::new();
    };
    for (_, extension) in items(list) {
        let fields = items(extension);
        if fields.first().map(|(_, oid)| *oid) != Some(OID_SUBJECT_ALT_NAME) {
            continue;
        }
        let Some((_, names, _)) = fields
            .iter()
            .find(|(tag, _)| *tag == 0x04)
            .and_then(|(_, value)| der(value))
        else {
            continue;
        };
        return items(names)
            .into_iter()
            .filter_map(|(tag, name)| match tag {
                0x82 => Some(String::from_utf8_lossy(name).into_owned()),
                0x87 => match <[u8; 4]>::try_from(name) {
                    Ok(v4) => Some(Ipv4Addr::from(v4).to_string()),
                    Err(_) => Some(Ipv6Addr::from(<[u8; 16]>::try_from(name).ok()?).to_string()),
                },
                _ => None,
            })
            .collect();
    }
    Vec::new()
}

/// Subject, issuer, names and validity of a DER certificate.
pub fn parse_certificate(der_bytes: &[u8], now_ms: u64) -> Option<TlsInfo> {
    let (_, certificate, _) = der(der_bytes)?;
    let (_, tbs, _) = der(certificate)?;
    let mut fields = items(tbs).into_iter().peekable();
    fields.next_if(|(tag, _)| *tag == 0xa0);
    let _serial = fields.next()?;
    let _signature = fields.next()?;
    let (_, issuer) = fields.next()?;
    let (_, validity) = fields.next()?;
    let (_, subject) = fields.next()?;
    let names = fields
        .find(|(tag, _)| *tag == 0xa3)
        .map(|(_, content)| alt_names(content))
        .unwrap_or_default();
    let times = items(validity);
    let not_before = times.first().and_then(|(tag, c)| time(*tag, c));
    let not_after = times.get(1).and_then(|(tag, c)| time(*tag, c));
    Some(TlsInfo {
        subject: name_attribute(subject, OID_COMMON_NAME),
        issuer: name_attribute(issuer, OID_COMMON_NAME)
            .or_else(|| name_attribute(issuer, OID_ORGANIZATION)),
        names,
        not_before,
        not_after,
        days_left: not_after.map(|end| (end as i64 - now_ms as i64).div_euclid(DAY_MS)),
        self_signed: issuer == subject,
        version: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if content.len() < 0x80 {
            out.push(content.len() as u8);
        } else {
            out.push(0x82);
            out.extend_from_slice(&(content.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(content);
        out
    }

    fn name(oid: &[u8], value: &[u8]) -> Vec<u8> {
        let pair = tlv(0x30, &[tlv(0x06, oid), tlv(0x0c, value)].concat());
        tlv(0x30, &tlv(0x31, &pair))
    }

    /// A certificate assembled from its parts, valid 2025-01-01 to 2035-01-01.
    fn certificate(issuer: Vec<u8>, subject: Vec<u8>) -> Vec<u8> {
        let validity = tlv(
            0x30,
            &[tlv(0x17, b"250101000000Z"), tlv(0x18, b"20350101000000Z")].concat(),
        );
        let san = tlv(
            0x30,
            &[tlv(0x82, b"localhost"), tlv(0x87, &[127, 0, 0, 1])].concat(),
        );
        let extension = tlv(
            0x30,
            &[tlv(0x06, OID_SUBJECT_ALT_NAME), tlv(0x04, &san)].concat(),
        );
        let algorithm = tlv(0x30, &tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 4, 3, 2]));
        let tbs = tlv(
            0x30,
            &[
                tlv(0xa0, &tlv(0x02, &[2])),
                tlv(0x02, &[1]),
                algorithm.clone(),
                issuer,
                validity,
                subject,
                tlv(0x30, &[0; 200]),
                tlv(0xa3, &tlv(0x30, &extension)),
            ]
            .concat(),
        );
        tlv(0x30, &[tbs, algorithm, tlv(0x03, &[0; 10])].concat())
    }

    #[test]
    fn certificates() {
        let start = 1_735_689_600_000;
        let local = name(OID_COMMON_NAME, b"localhost");
        let info = parse_certificate(
            &certificate(local.clone(), local.clone()),
            start + 10 * DAY_MS as u64,
        )
        .unwrap();
        assert_eq!(info.subject.as_deref(), Some("localhost"));
        assert_eq!(info.names, vec!["localhost", "127.0.0.1"]);
        assert!(info.self_signed);
        assert_eq!(info.not_before, Some(start));
        assert_eq!(info.not_after, Some(2_051_222_400_000));
        assert_eq!(info.days_left, Some(3652 - 10));

        let issuer = name(OID_ORGANIZATION, b"Let's Encrypt");
        let info = parse_certificate(&certificate(issuer, local), 2_051_222_400_000 + 1).unwrap();
        assert_eq!(info.issuer.as_deref(), Some("Let's Encrypt"));
        assert!(!info.self_signed);
        assert_eq!(info.days_left, Some(-1));
        assert!(parse_certificate(b"\x30\x05junk", start).is_none());
    }

    #[test]
    fn times() {
        assert_eq!(time(0x17, b"700101000000Z"), Some(0));
        assert_eq!(time(0x17, b"250101000000Z"), Some(1_735_689_600_000));
        assert_eq!(time(0x18, b"20500101000000Z"), Some(2_524_608_000_000));
        assert_eq!(time(0x17, b"2501010000Z"), None);
    }
}
