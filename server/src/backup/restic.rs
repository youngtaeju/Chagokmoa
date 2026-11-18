//! restic 하위 프로세스 래퍼. 저장소 위치와 비밀값은 환경 변수로만(명령줄 인자 금지).
//!
//! restic 0.16 ~ 0.18 공통 동작만 사용. 버전 차이
//! - 저장소 없음: 0.17 부터 종료 코드 10, 이전은 1 + 안내 문구
//! - 키 틀림: 0.17 부터 종료 코드 12, 이전은 1 + `wrong password`
//! - `--json` 치명 오류: 0.17 부터 stderr 에 `exit_error` JSON, 이전은 텍스트
//! - 항목 오류 `error`: 0.17 부터 `{message}`, 이전은 Go 구조체 그대로(빈 객체일 때도 있음)

use std::{
    collections::{BTreeSet, VecDeque},
    ffi::OsString,
    fmt,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    process::Command,
};

use super::{mask_location, split_url_password};
use crate::error::{AppError, AppResult};

/// 컨테이너마다 바뀌는 호스트 이름 대신 고정. 이전 스냅숏을 부모로 찾고 보관 정책 묶음도 하나.
const HOST: &str = "chagokmoa";
/// 접속 확인 수준(probe, init, unlock). 원격 재시도로 오래 멈추는 경우 대비.
const QUICK_TIMEOUT: Duration = Duration::from_secs(120);
/// 목록과 보관 정책. 원격 첫 조회는 캐시가 비어 느림.
const LIST_TIMEOUT: Duration = Duration::from_secs(600);
/// restic 이 바이너리만으로 지원하는 원격 접두어. 그 밖은 로컬 경로.
const REMOTE_PREFIXES: [&str; 8] = [
    "s3:", "b2:", "rest:", "azure:", "gs:", "sftp:", "swift:", "rclone:",
];
/// 빈 폴더 판정 때 무시하는 NAS 와 파일시스템 부산물. `.` 시작 이름도 무시.
const IGNORED_ENTRIES: [&str; 4] = ["@eaDir", "#recycle", "#snapshot", "lost+found"];
/// 오류 문구에 남기는 stderr 줄 수.
const MESSAGE_TAIL: usize = 20;

/// `Debug` 는 비밀값 없이(아래 수동 구현).
#[derive(Clone)]
pub struct Repo {
    /// 로컬 경로 또는 restic 원격 주소.
    pub location: String,
    /// `RESTIC_PASSWORD` 와 원격 인증값.
    pub env: Vec<(String, String)>,
    /// `{DATA_DIR}/cache/restic`. 백업 대상 아님.
    pub cache_dir: PathBuf,
}

impl fmt::Debug for Repo {
    /// 위치의 userinfo 비밀번호는 `***`, env 는 키만.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = mask_location(&self.location);
        let env_keys: Vec<&str> = self.env.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("Repo")
            .field("location", &location)
            .field("env_keys", &env_keys)
            .field("cache_dir", &self.cache_dir)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    /// 폴더 없음 또는 비어 있음(원격은 저장소 없음).
    Empty,
    /// 키로 열림.
    Restic,
    /// restic 저장소 아닌 내용(로컬만 판정).
    Foreign,
    WrongKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retention {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupSummary {
    pub snapshot_id: String,
    pub files_new: u64,
    pub data_added: u64,
    pub duration: Duration,
    /// 종료 코드 3 이지만 originals 의 사라진 파일뿐일 때.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub short_id: String,
    pub time: DateTime<Utc>,
    pub hostname: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub paths: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub tags: Vec<String>,
}

fn null_as_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Option::<Vec<String>>::deserialize(d).map(Option::unwrap_or_default)
}

/// `backup --json` 의 항목 오류.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemError {
    pub item: Option<PathBuf>,
    pub message: String,
}

#[derive(Deserialize)]
struct Summary {
    snapshot_id: String,
    files_new: u64,
    data_added: u64,
    total_duration: f64,
}

/// 끝난 프로세스. stdout 은 호출자가 줄 단위로 처리.
struct Finished {
    code: Option<i32>,
    errors: Vec<ItemError>,
    /// 항목 오류 외 stderr 마지막 줄들. `exit_error` 는 문구만.
    messages: Vec<String>,
}

impl Finished {
    fn success(&self) -> bool {
        self.code == Some(0)
    }

    fn mentions(&self, needle: &str) -> bool {
        self.messages.iter().any(|m| m.contains(needle))
    }

    fn repo_missing(&self) -> bool {
        self.code == Some(10)
            || self.mentions("Is there a repository at the following location?")
            || self.mentions("repository does not exist")
    }

    fn wrong_key(&self) -> bool {
        self.code == Some(12) || self.mentions("wrong password")
    }
}

/// `{"message_type":"error",...}` 한 줄. 그 밖은 None.
pub fn parse_item_error(line: &str) -> Option<ItemError> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v["message_type"] != "error" {
        return None;
    }
    let err = &v["error"];
    let message = err["message"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            // 0.16: {"Op":"lstat","Path":"...","Err":2}
            let op = err["Op"].as_str()?;
            let errno = err["Err"].as_i64()?;
            let errno = i32::try_from(errno).ok()?;
            Some(format!(
                "{op}: {}",
                std::io::Error::from_raw_os_error(errno)
            ))
        })
        .unwrap_or_else(|| "unreadable".to_owned());
    Some(ItemError {
        item: v["item"].as_str().map(PathBuf::from),
        message,
    })
}

/// 절대 경로로 바꾸고 `.`, `..` 를 글자 단위로 정리. restic 이 보고하는 항목 경로와 같은 형태(심볼릭 링크 해석 없음).
fn clean_absolute(path: &Path) -> std::io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in std::path::absolute(path)?.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

/// 종료 코드 3 판정. originals 아래에서 지금은 없는 항목뿐이면 경고 문구, 그 밖은 실패 문구.
/// 상대 경로 대상도 restic 은 항목을 정리된 절대 경로로 보고하므로 뿌리도 같은 형태로.
pub fn exit3_outcome(paths: &[PathBuf], errors: &[ItemError]) -> Result<String, String> {
    let roots: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| clean_absolute(p).ok())
        .filter(|p| p.file_name().is_some_and(|n| n == "originals"))
        .collect();
    let vanished = |e: &ItemError| {
        e.item.as_deref().is_some_and(|item| {
            roots
                .iter()
                .any(|r| item.starts_with(r) && item != r.as_path())
                && std::fs::symlink_metadata(item)
                    .is_err_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        })
    };
    if let Some(e) = errors.iter().find(|e| !vanished(e)) {
        let item = e
            .item
            .as_deref()
            .map_or_else(String::new, |p| format!("{}: ", p.display()));
        return Err(format!("source unreadable: {item}{}", e.message));
    }
    if errors.is_empty() {
        return Err("source unreadable".to_owned());
    }
    let count = errors
        .iter()
        .filter_map(|e| e.item.as_deref())
        .collect::<BTreeSet<_>>()
        .len();
    Ok(format!("{count} files vanished during backup"))
}

/// 스냅숏 id 또는 `latest`. 옵션으로 해석될 값 차단.
fn valid_snapshot_id(id: &str) -> bool {
    id == "latest"
        || (!id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// 줄 단위 읽기. 잘못된 UTF-8 은 대체 문자로.
async fn for_each_line(
    mut reader: impl AsyncBufRead + Unpin,
    mut f: impl FnMut(&str),
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).await? == 0 {
            return Ok(());
        }
        f(String::from_utf8_lossy(&buf).trim_end());
    }
}

fn stderr_line(line: &str, errors: &mut Vec<ItemError>, messages: &mut VecDeque<String>) {
    if let Some(e) = parse_item_error(line) {
        errors.push(e);
        return;
    }
    let message = match serde_json::from_str::<Value>(line) {
        Ok(v) if v["message_type"] == "exit_error" => {
            v["message"].as_str().unwrap_or_default().to_owned()
        }
        Ok(v) if v.is_object() => return,
        _ => line.trim().to_owned(),
    };
    if message.is_empty() {
        return;
    }
    if messages.len() == MESSAGE_TAIL {
        messages.pop_front();
    }
    messages.push_back(message);
}

impl Repo {
    /// 로컬 저장소 폴더. 원격이면 None.
    fn local_dir(&self) -> Option<PathBuf> {
        if REMOTE_PREFIXES.iter().any(|p| self.location.starts_with(p)) {
            return None;
        }
        let path = self
            .location
            .strip_prefix("local:")
            .unwrap_or(&self.location);
        Some(PathBuf::from(path))
    }

    /// `restic` 명령. 상속된 `RESTIC_*` 제거 후 위치와 비밀값은 환경 변수로.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("restic");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESTIC_") {
                cmd.env_remove(key);
            }
        }
        cmd.env("RESTIC_REPOSITORY", &self.location)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .arg("--cache-dir")
            .arg(&self.cache_dir)
            .args(args);
        cmd
    }

    /// 오류 문구에서 비밀값 가림.
    pub(crate) fn redact(&self, text: &str) -> String {
        let (_, location_password) = split_url_password(&self.location);
        let mut out = text.to_owned();
        for secret in self
            .env
            .iter()
            .map(|(_, v)| v.as_str())
            .chain(location_password.as_deref())
            .filter(|s| s.len() >= 4)
        {
            out = out.replace(secret, "***");
        }
        out
    }

    fn failure(&self, what: &str, f: &Finished) -> AppError {
        let code = f
            .code
            .map_or_else(|| "signal".to_owned(), |c| c.to_string());
        let tail: Vec<&str> = f
            .messages
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(String::as_str)
            .collect();
        AppError::Internal(anyhow!(
            "restic {what} failed (exit {code}): {}",
            self.redact(&tail.join(" / "))
        ))
    }

    /// 실행 후 종료 상태와 stderr 수집. `timeout` 없으면 호출자 future drop 때까지(kill_on_drop).
    async fn run(
        &self,
        args: &[&str],
        timeout: Option<Duration>,
        on_stdout: impl FnMut(&str),
    ) -> AppResult<Finished> {
        let what = args.first().copied().unwrap_or_default();
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .context("restic cache dir create failed")?;
        let mut cmd = self.command(args);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().context("restic spawn failed")?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            return Err(anyhow!("restic pipes missing").into());
        };
        let work = async move {
            let mut errors = Vec::new();
            let mut messages = VecDeque::new();
            let (out, err) = tokio::join!(
                for_each_line(BufReader::new(stdout), on_stdout),
                for_each_line(BufReader::new(stderr), |l| {
                    stderr_line(l, &mut errors, &mut messages)
                }),
            );
            out?;
            err?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>(Finished {
                code: status.code(),
                errors,
                messages: messages.into(),
            })
        };
        let finished = match timeout {
            Some(t) => tokio::time::timeout(t, work)
                .await
                .map_err(|_| anyhow!("restic {what} timed out after {}s", t.as_secs()))?,
            None => work.await,
        }
        .with_context(|| format!("restic {what} io failed"))?;
        Ok(finished)
    }

    /// 성공만 허용하고 stdout 은 버림.
    async fn run_ok(&self, args: &[&str], timeout: Option<Duration>) -> AppResult<()> {
        let f = self.run(args, timeout, |_| {}).await?;
        if !f.success() {
            return Err(self.failure(args[0], &f));
        }
        Ok(())
    }

    pub async fn probe(&self) -> AppResult<RepoState> {
        if let Some(dir) = self.local_dir() {
            match local_layout(&dir).await? {
                Layout::Empty => return Ok(RepoState::Empty),
                Layout::Foreign => return Ok(RepoState::Foreign),
                Layout::Restic => {}
            }
        }
        // 읽기만. 진행 중 prune 의 배타 잠금에 막히지 않게 잠금 없이
        let f = self
            .run(&["cat", "config", "--no-lock"], Some(QUICK_TIMEOUT), |_| {})
            .await?;
        if f.success() {
            Ok(RepoState::Restic)
        } else if f.wrong_key() {
            Ok(RepoState::WrongKey)
        } else if f.repo_missing() {
            Ok(RepoState::Empty)
        } else {
            Err(self.failure("cat", &f))
        }
    }

    pub async fn init(&self) -> AppResult<()> {
        self.run_ok(&["init"], Some(QUICK_TIMEOUT)).await
    }

    pub async fn unlock(&self) -> AppResult<()> {
        self.run_ok(&["unlock"], Some(QUICK_TIMEOUT)).await
    }

    pub async fn prune(&self) -> AppResult<()> {
        self.run_ok(&["prune"], None).await
    }

    pub async fn check(&self) -> AppResult<()> {
        self.run_ok(&["check"], None).await
    }

    /// 경로가 하나라도 없으면 실패(restic 은 없는 경로를 건너뛰고 성공 처리).
    /// 대상은 정리된 절대 경로로 넘김(스냅숏 경로가 작업 디렉터리와 무관).
    pub async fn backup(&self, paths: &[PathBuf], tag: &str) -> AppResult<BackupSummary> {
        let paths = paths
            .iter()
            .map(|p| clean_absolute(p))
            .collect::<std::io::Result<Vec<_>>>()
            .context("backup source path invalid")?;
        for path in &paths {
            tokio::fs::symlink_metadata(path)
                .await
                .with_context(|| format!("backup source missing: {}", path.display()))?;
        }
        let path_args: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut args = vec!["backup", "--json", "--host", HOST, "--tag", tag];
        args.extend(path_args.iter().map(String::as_str));

        let mut summary = None;
        let f = self
            .run(&args, None, |line| {
                if line.contains(r#""message_type":"summary""#) {
                    summary = serde_json::from_str::<Summary>(line).ok();
                }
            })
            .await?;
        let warning = match f.code {
            Some(0) => None,
            Some(3) => Some(exit3_outcome(&paths, &f.errors).map_err(|reason| {
                AppError::Internal(anyhow!(
                    "restic backup failed (exit 3): {}",
                    self.redact(&reason)
                ))
            })?),
            _ => return Err(self.failure("backup", &f)),
        };
        let s = summary.context("restic backup summary missing")?;
        Ok(BackupSummary {
            snapshot_id: s.snapshot_id,
            files_new: s.files_new,
            data_added: s.data_added,
            duration: Duration::try_from_secs_f64(s.total_duration).unwrap_or_default(),
            warning,
        })
    }

    /// 앱이 만든 스냅숏(고정 호스트)만 한 묶음으로. 모두 0 이면 지우지 않음.
    pub async fn forget(&self, policy: Retention) -> AppResult<()> {
        let keeps = [
            ("--keep-daily", policy.daily),
            ("--keep-weekly", policy.weekly),
            ("--keep-monthly", policy.monthly),
        ];
        let counts: Vec<(&str, String)> = keeps
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(flag, n)| (*flag, n.to_string()))
            .collect();
        if counts.is_empty() {
            return Ok(());
        }
        let mut args = vec!["forget", "--json", "--host", HOST, "--group-by", "host"];
        for (flag, n) in &counts {
            args.extend([*flag, n.as_str()]);
        }
        self.run_ok(&args, Some(LIST_TIMEOUT)).await
    }

    /// 오래된 순.
    pub async fn snapshots(&self) -> AppResult<Vec<Snapshot>> {
        let mut out = String::new();
        let f = self
            .run(&["snapshots", "--json"], Some(LIST_TIMEOUT), |l| {
                out.push_str(l)
            })
            .await?;
        if !f.success() {
            return Err(self.failure("snapshots", &f));
        }
        let mut list: Vec<Snapshot> =
            serde_json::from_str(&out).context("restic snapshots parse failed")?;
        list.sort_by_key(|s| s.time);
        Ok(list)
    }

    /// `target` 아래 스냅숏 절대 경로 그대로 복원.
    pub async fn restore(&self, snapshot: &str, target: &Path) -> AppResult<()> {
        if !valid_snapshot_id(snapshot) {
            return Err(anyhow!("invalid snapshot id").into());
        }
        let target = target.to_string_lossy();
        self.run_ok(&["restore", snapshot, "--target", &target], None)
            .await
    }
}

enum Layout {
    Empty,
    Restic,
    Foreign,
}

/// 로컬 폴더 내용으로 저장소 여부 판정. restic 저장소는 `config` 파일과 `keys` 폴더.
async fn local_layout(dir: &Path) -> AppResult<Layout> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Layout::Empty),
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return Ok(Layout::Foreign),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("backup dir unreadable: {}", dir.display()))
                .into());
        }
    };
    let (mut any, mut config, mut keys) = (false, false, false);
    while let Some(entry) = entries
        .next_entry()
        .await
        .context("backup dir unreadable")?
    {
        let name: OsString = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || IGNORED_ENTRIES.contains(&name.as_ref()) {
            continue;
        }
        any = true;
        let file_type = entry.file_type().await.context("backup dir unreadable")?;
        match name.as_ref() {
            "config" if file_type.is_file() => config = true,
            "keys" if file_type.is_dir() => keys = true,
            _ => {}
        }
    }
    Ok(match (any, config && keys) {
        (false, _) => Layout::Empty,
        (true, true) => Layout::Restic,
        (true, false) => Layout::Foreign,
    })
}
