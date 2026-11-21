use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// 세션 토큰 생성. (원문 base64url 43자, SHA-256 해시)
pub fn generate() -> (String, Vec<u8>) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let raw = URL_SAFE_NO_PAD.encode(bytes);
    let hashed = hash(&raw);
    (raw, hashed)
}

pub fn hash(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

fn mac(secret: &[u8], payload: &str) -> HmacSha256 {
    // HMAC 은 임의 길이 키 허용. new_from_slice 는 실패하지 않음.
    let mut mac = HmacSha256::new_from_slice(secret).unwrap_or_else(|_| unreachable!());
    mac.update(payload.as_bytes());
    mac
}

/// 형식 `<base64url payload>.<base64url hmac-sha256>`. MAC 대상은 `<context>:<base64url payload>`.
/// `context` 로 용도 분리(다른 서명 값의 재사용 차단). 만료 등 내용 검사는 호출자.
pub fn sign_opaque(secret: &[u8], context: &str, payload: &[u8]) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(payload);
    let sig = mac(secret, &format!("{context}:{encoded}")).finalize();
    format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(sig.into_bytes()))
}

/// 형식 오류, MAC 불일치는 `None`. MAC 비교는 상수 시간(`verify_slice`).
pub fn verify_opaque(secret: &[u8], context: &str, token: &str) -> Option<Vec<u8>> {
    let (encoded, sig) = token.split_once('.')?;
    let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    mac(secret, &format!("{context}:{encoded}"))
        .verify_slice(&sig)
        .ok()?;
    URL_SAFE_NO_PAD.decode(encoded).ok()
}

/// 형식 `<uuid>.<unix exp>.<base64url hmac-sha256>`. MAC 은 앞 두 파트 문자열 전체 대상.
pub fn sign_media(secret: &[u8], user_id: Uuid, exp: DateTime<Utc>) -> String {
    let payload = format!("{user_id}.{}", exp.timestamp());
    let sig = URL_SAFE_NO_PAD.encode(mac(secret, &payload).finalize().into_bytes());
    format!("{payload}.{sig}")
}

/// 형식 오류, MAC 불일치, 만료(`now >= exp`)는 `None`.
pub fn verify_media(secret: &[u8], token: &str, now: DateTime<Utc>) -> Option<Uuid> {
    let mut parts = token.split('.');
    let (id, exp, sig) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let user_id = Uuid::parse_str(id).ok()?;
    let exp_ts: i64 = exp.parse().ok()?;
    let sig = URL_SAFE_NO_PAD.decode(sig).ok()?;
    mac(secret, &format!("{id}.{exp}"))
        .verify_slice(&sig)
        .ok()?;
    (now.timestamp() < exp_ts).then_some(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn generate_hash_matches() {
        let (raw, h) = generate();
        assert_eq!(hash(&raw), h);
        assert_eq!(raw.len(), 43);
        assert_eq!(h.len(), 32);
        assert_ne!(generate().0, raw);
    }

    #[test]
    fn media_token_roundtrip_and_expiry() {
        let secret = b"secret";
        let id = Uuid::now_v7();
        let now = Utc::now();
        let exp = now + Duration::seconds(60);
        let t = sign_media(secret, id, exp);

        assert_eq!(verify_media(secret, &t, now), Some(id));
        assert_eq!(
            verify_media(secret, &t, exp - Duration::seconds(1)),
            Some(id)
        );
        assert_eq!(verify_media(secret, &t, exp), None);
        assert_eq!(verify_media(secret, &t, exp + Duration::seconds(1)), None);
        assert_eq!(verify_media(b"other", &t, now), None);
    }

    #[test]
    fn media_token_rejects_tampering() {
        let secret = b"secret";
        let id = Uuid::now_v7();
        let now = Utc::now();
        let exp = now + Duration::seconds(60);
        let t = sign_media(secret, id, exp);
        let parts: Vec<&str> = t.split('.').collect();

        let other = Uuid::now_v7();
        let forged_id = format!("{other}.{}.{}", parts[1], parts[2]);
        assert_eq!(verify_media(secret, &forged_id, now), None);

        let forged_exp = format!("{}.{}.{}", parts[0], exp.timestamp() + 3600, parts[2]);
        assert_eq!(verify_media(secret, &forged_exp, now), None);
    }

    #[test]
    fn opaque_roundtrip_and_tampering() {
        let secret = b"secret";
        let t = sign_opaque(secret, "ctx", b"{\"a\":1}");
        assert_eq!(
            verify_opaque(secret, "ctx", &t).as_deref(),
            Some(&b"{\"a\":1}"[..])
        );
        assert_eq!(verify_opaque(b"other", "ctx", &t), None);
        assert_eq!(verify_opaque(secret, "other", &t), None);

        let (_, sig) = t.split_once('.').unwrap();
        let forged = format!("{}.{sig}", URL_SAFE_NO_PAD.encode(b"{\"a\":2}"));
        assert_eq!(verify_opaque(secret, "ctx", &forged), None);
        for bad in ["", "abc", ".", &format!("{t}.x"), &format!("{t}x")] {
            assert_eq!(verify_opaque(secret, "ctx", bad), None, "{bad}");
        }
    }

    #[test]
    fn media_token_rejects_malformed() {
        let secret = b"secret";
        let now = Utc::now();
        let t = sign_media(secret, Uuid::now_v7(), now + Duration::seconds(60));
        let parts: Vec<&str> = t.split('.').collect();

        for bad in [
            "",
            "a.b",
            &format!("{t}.extra"),
            &format!("not-a-uuid.{}.{}", parts[1], parts[2]),
            &format!("{}.abc.{}", parts[0], parts[2]),
            &format!("{}.{}.!!!", parts[0], parts[1]),
            &format!("{}.{}.", parts[0], parts[1]),
        ] {
            assert_eq!(verify_media(secret, bad, now), None, "{bad}");
        }
    }
}
