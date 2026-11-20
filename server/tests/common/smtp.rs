//! 테스트용 SMTP 수신기. 127.0.0.1 평문, 발송 확인에 필요한 명령만.

use std::{
    future,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use chagokmoa::mail::{MailConfig, MailSecurity, settings};
use sqlx::PgPool;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, tcp::OwnedWriteHalf},
    task::JoinHandle,
};

#[derive(Clone)]
pub enum Mode {
    Accept,
    /// `MAIL FROM` 에 이 코드와 문구로 거부.
    Reject(u16, String),
    /// 접속만 받고 무응답.
    Hang,
}

#[derive(Clone, Debug)]
pub struct ReceivedMail {
    pub from: String,
    pub to: Vec<String>,
    /// 점 복원 후 본문(헤더 포함), 줄바꿈 `\r\n`.
    pub data: String,
    /// `AUTH` 로 받은 (사용자 이름, 비밀번호).
    pub auth: Option<(String, String)>,
}

impl ReceivedMail {
    /// 헤더 뒤 본문. `Content-Transfer-Encoding` 에 따라 복원.
    pub fn body(&self) -> String {
        let (headers, body) = self.data.split_once("\r\n\r\n").expect("header end");
        let encoding = headers
            .lines()
            .find_map(|l| l.strip_prefix("Content-Transfer-Encoding:"))
            .map_or("7bit", str::trim)
            .to_ascii_lowercase();
        match encoding.as_str() {
            "7bit" | "8bit" => body.to_owned(),
            "base64" => decode(&body.replace("\r\n", "")),
            "quoted-printable" => quoted_printable(body),
            other => panic!("unsupported encoding {other}"),
        }
    }
}

/// 소프트 줄바꿈 제거 후 `=XX` 복원.
fn quoted_printable(text: &str) -> String {
    let joined = text.replace("=\r\n", "");
    let bytes = joined.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("qp hex");
            out.push(u8::from_str_radix(hex, 16).expect("qp hex"));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf8")
}

/// 수신기로 보내는 메일 설정 저장. 암호화, 인증 없음.
pub async fn configure(pool: &PgPool, port: u16) {
    settings::save(
        pool,
        &MailConfig {
            host: "127.0.0.1".into(),
            port,
            security: MailSecurity::None,
            username: None,
            password: None,
            from_address: "noreply@example.com".into(),
        },
    )
    .await
    .expect("save mail settings");
}

pub struct Receiver {
    pub port: u16,
    mails: Arc<Mutex<Vec<ReceivedMail>>>,
    task: JoinHandle<()>,
}

impl Receiver {
    pub async fn start(mode: Mode) -> Receiver {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind smtp");
        let port = listener.local_addr().expect("smtp addr").port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let shared = mails.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mode = mode.clone();
                let mails = shared.clone();
                tokio::spawn(async move {
                    if let Mode::Hang = mode {
                        let _open = stream;
                        future::pending::<()>().await;
                    } else {
                        // 상대가 끊으면 오류로 종료. 수신 기록과 무관.
                        let _ = session(stream, mode, mails).await;
                    }
                });
            }
        });
        Receiver { port, mails, task }
    }

    pub fn messages(&self) -> Vec<ReceivedMail> {
        self.mails.lock().expect("smtp mails").clone()
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn reply(w: &mut OwnedWriteHalf, text: &str) -> std::io::Result<()> {
    w.write_all(text.as_bytes()).await?;
    w.write_all(b"\r\n").await
}

fn decode(b64: &str) -> String {
    String::from_utf8(STANDARD.decode(b64.trim()).expect("base64")).expect("utf8")
}

/// `<addr>` 안쪽. 뒤 파라미터 무시.
fn angle(arg: &str) -> String {
    let start = arg.find('<').map_or(0, |i| i + 1);
    let end = arg[start..].find('>').map_or(arg.len(), |i| start + i);
    arg[start..end].to_owned()
}

async fn session(
    stream: TcpStream,
    mode: Mode,
    mails: Arc<Mutex<Vec<ReceivedMail>>>,
) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut next = async || -> std::io::Result<String> {
        lines
            .next_line()
            .await?
            .ok_or(std::io::ErrorKind::UnexpectedEof.into())
    };
    reply(&mut w, "220 localhost test smtp").await?;
    let mut auth = None;
    let mut from = String::new();
    let mut to = Vec::new();
    loop {
        let line = next().await?;
        let (verb, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match verb.to_ascii_uppercase().as_str() {
            "EHLO" | "HELO" => reply(&mut w, "250-localhost\r\n250 AUTH PLAIN LOGIN").await?,
            "AUTH" => {
                let (mechanism, initial) = arg.split_once(' ').unwrap_or((arg, ""));
                let creds = match mechanism.to_ascii_uppercase().as_str() {
                    "PLAIN" => {
                        let encoded = if initial.is_empty() {
                            reply(&mut w, "334 ").await?;
                            next().await?
                        } else {
                            initial.to_owned()
                        };
                        let plain = decode(&encoded);
                        let mut parts = plain.split('\0').skip(1);
                        let user = parts.next().unwrap_or_default().to_owned();
                        let pass = parts.next().unwrap_or_default().to_owned();
                        (user, pass)
                    }
                    "LOGIN" => {
                        let user = if initial.is_empty() {
                            reply(&mut w, "334 VXNlcm5hbWU6").await?;
                            decode(&next().await?)
                        } else {
                            decode(initial)
                        };
                        reply(&mut w, "334 UGFzc3dvcmQ6").await?;
                        (user, decode(&next().await?))
                    }
                    _ => {
                        reply(&mut w, "504 unsupported").await?;
                        continue;
                    }
                };
                auth = Some(creds);
                reply(&mut w, "235 authenticated").await?;
            }
            "MAIL" => {
                if let Mode::Reject(code, text) = &mode {
                    reply(&mut w, &format!("{code} {text}")).await?;
                    continue;
                }
                from = angle(arg);
                to.clear();
                reply(&mut w, "250 ok").await?;
            }
            "RCPT" => {
                to.push(angle(arg));
                reply(&mut w, "250 ok").await?;
            }
            "DATA" => {
                reply(&mut w, "354 end with .").await?;
                let mut data = Vec::new();
                loop {
                    let l = next().await?;
                    if l == "." {
                        break;
                    }
                    data.push(l.strip_prefix('.').unwrap_or(&l).to_owned());
                }
                mails.lock().expect("smtp mails").push(ReceivedMail {
                    from: from.clone(),
                    to: to.clone(),
                    data: data.join("\r\n"),
                    auth: auth.clone(),
                });
                reply(&mut w, "250 queued").await?;
            }
            "RSET" | "NOOP" => reply(&mut w, "250 ok").await?,
            "QUIT" => {
                reply(&mut w, "221 bye").await?;
                return Ok(());
            }
            _ => reply(&mut w, "500 unknown command").await?,
        }
    }
}
