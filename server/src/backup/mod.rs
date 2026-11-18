//! 백업. restic 저장소와 DB 덤프.

pub mod dump;
pub mod restic;

/// URL userinfo 의 비밀번호 분리. (비밀번호 뺀 URL, 인코딩 그대로인 비밀번호).
fn split_url_password(url: &str) -> (String, Option<String>) {
    let Some(scheme_end) = url.find("://").map(|i| i + 3) else {
        return (url.to_owned(), None);
    };
    let rest = &url[scheme_end..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let Some(at) = authority.rfind('@') else {
        return (url.to_owned(), None);
    };
    let Some(colon) = authority[..at].find(':') else {
        return (url.to_owned(), None);
    };
    let password = authority[colon + 1..at].to_owned();
    let stripped = format!("{}{}", &url[..scheme_end + colon], &url[scheme_end + at..]);
    (stripped, Some(password))
}

/// `%XX` 해제. 잘못된 순서는 그대로.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_password() {
        assert_eq!(
            split_url_password("postgres://u:p%40w@h:5432/db?sslmode=require"),
            (
                "postgres://u@h:5432/db?sslmode=require".to_owned(),
                Some("p%40w".to_owned())
            )
        );
        assert_eq!(
            split_url_password("rest:https://u:p@host/repo"),
            ("rest:https://u@host/repo".to_owned(), Some("p".to_owned()))
        );
        assert_eq!(
            split_url_password("postgres://u@h/db"),
            ("postgres://u@h/db".to_owned(), None)
        );
        assert_eq!(
            split_url_password("s3:https://s3.example/bucket"),
            ("s3:https://s3.example/bucket".to_owned(), None)
        );
        // 경로 안의 @ 는 userinfo 아님
        assert_eq!(
            split_url_password("postgres://h/db@x:y"),
            ("postgres://h/db@x:y".to_owned(), None)
        );
    }

    #[test]
    fn decodes_percent() {
        assert_eq!(percent_decode("p%40w%3Ax"), "p@w:x");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%+1"), "%+1");
    }
}
