# 운영 안내

Docker Compose 로 설치한 차곡모아를 운영할 때의 안내입니다. 설치와 업그레이드는 [README](../README.md) 를 참고하십시오. 아래 명령은 `docker-compose.yml` 이 있는 디렉터리에서 실행하며, 서비스 이름은 `app`(애플리케이션)과 `postgres` 입니다.

## 리버스 프록시와 https

휴대폰에서 PWA 를 설치하고 서비스 워커를 동작시키려면 https 가 필요합니다(localhost 는 예외). 외부에서 접속하려면 리버스 프록시로 https 를 종단하고, `.env` 의 `PUBLIC_URL` 을 그 https 주소로 설정합니다. 프록시가 같은 호스트에 있다면 compose 의 `ports` 를 `127.0.0.1:${PORT:-8080}:8080` 으로 바꿔 외부 노출을 막습니다.

업로드는 8 MiB 단위 조각(tus)으로 전송되므로 요청 본문 상한은 16 MB 이상이면 충분합니다. 큰 영상은 업로드 완료(finalize) 시 서버가 해시를 계산하느라 응답이 오래 걸릴 수 있으므로 읽기 타임아웃을 넉넉하게 둡니다.

### nginx

```nginx
server {
    listen 443 ssl;
    server_name chagokmoa.example.com;

    # ssl_certificate, ssl_certificate_key 설정은 생략

    client_max_body_size 16m;
    client_body_timeout 300s;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_request_buffering off;
        proxy_read_timeout 300s;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-Host $host;
    }
}
```

### Caddy

Caddy 는 https 인증서를 자동으로 발급하며 `X-Forwarded-*` 헤더도 기본으로 설정합니다.

```caddyfile
chagokmoa.example.com {
    reverse_proxy 127.0.0.1:8080 {
        transport http {
            read_timeout 300s
        }
    }
}
```

### Cloudflare Tunnel

Cloudflare Tunnel 은 요청당 응답 대기 시간이 100초로 제한됩니다. 용량이 큰 영상의 finalize 가 이 제한을 넘기면 실패할 수 있으므로, 큰 영상을 자주 올린다면 Tunnel 대신 다른 방식을 권장합니다. 사진 위주라면 대체로 문제가 없습니다.

`PUBLIC_URL` 이 https 이면 세션 쿠키가 Secure 로 설정되므로, LAN 에서 `http://nas:8080` 같은 http 주소로 접속하면 로그인이 오류 표시 없이 실패합니다. 항상 https 주소로 접속하십시오.

## 데이터 디렉터리

컨테이너의 `DATA_DIR` 은 `/data` 이며 호스트의 `./data` 에 연결됩니다. 하위 디렉터리는 다음과 같습니다.

| 디렉터리 | 내용 |
| --- | --- |
| `originals/` | 업로드한 원본 파일. 바이트 그대로 보존하며, 휴지통에서 영구 삭제된 항목(직접 삭제하거나 보관 기간이 지난 항목) 외에는 원본을 삭제하지 않습니다. |
| `derivatives/` | 썸네일, 변환본, 포스터 등 파생물. 원본에서 다시 만들 수 있어 백업하지 않습니다([파생물 재생성](#파생물-재생성)). |
| `tmp/` | 업로드 중인 임시 파일 |
| `orphans/` | DB 에 기록이 없는 고아 원본(아래 참조) |
| `cache/` | restic 캐시와 백업 중의 DB 덤프. 백업하지 않습니다. |

`DATA_DIR` 은 하나의 파일시스템 안에 있어야 합니다. 서버가 파일을 `rename` 으로 옮기므로, 하위 디렉터리를 서로 다른 볼륨이나 마운트로 나누면 동작하지 않습니다.

`./data` 의 소유자는 `PUID`/`PGID` 로 지정한 사용자입니다. 컨테이너는 시작할 때 소유권을 맞춘 뒤 해당 사용자로 서버를 실행합니다.

### 고아 원본(`orphans/`)

서버는 기동 직후와 이후 하루마다 정리 작업을 수행합니다. 이때 DB 에 행이 없는 원본 파일 중 24시간이 지난 것을 `DATA_DIR/orphans/originals/YYYY/MM/` 으로 옮깁니다. 크래시 등으로 업로드가 중단되었을 때, 또는 파일보다 오래된 DB 를 복원했을 때 생길 수 있습니다. 파일을 삭제하지는 않으며, assets 테이블이 비어 있으면 이 작업을 건너뜁니다. 이때 원본 파일이 남아 있으면 다른 DB 에 연결된 것일 수 있어 `assets table is empty, skipping derivatives and originals sweep` WARN 로그를 남깁니다.

`orphans/` 의 파일은 직접 확인한 뒤 필요 없으면 삭제하십시오. 소중한 사진이 있다면 다시 업로드해 복구할 수 있습니다.

## 메모리

이미지 처리는 원본 해상도로 디코드하므로 해상도에 비례해 메모리를 씁니다(릴리스 빌드 측정값, 사진 1장 처리 기준).

| 해상도 | 최대 메모리 |
| --- | --- |
| 48 MP | 약 300 MB |
| 200 MP (JPEG/PNG) | 약 0.74 GB |
| 200 MP, EXIF 방향에 따라 90도 회전이 필요한 경우 | 약 1.18 GB |

210,000,000 픽셀(200 MP 급)을 넘는 이미지는 디코드 전에 거부되어 해당 항목이 `failed` 상태가 됩니다.

동시에 처리하는 작업 수는 `WORKER_CONCURRENCY`(1~16, 기본 2)로 조절합니다. 초대형 이미지 두 장을 동시에 처리하면 약 2.4~3 GB 가 필요할 수 있으므로, 메모리 2~4 GB 의 NAS 에서는 `1` 또는 `2` 를 권장합니다.

영상은 ffprobe/ffmpeg 하위 프로세스로 처리하며 시간 제한이 있습니다(분석 30초, 포스터 60초, 재포장 120초 + 영상 길이, 변환 300초 + 영상 길이의 4배). 제한을 넘기면 해당 항목이 `failed` 가 됩니다. 색 태그(transfer, primaries, matrix)가 모두 있는 HDR(HLG/PQ) 영상은 SDR 로 톤매핑하고, 태그가 불완전한 영상은 톤매핑 없이 변환합니다.

## 백업

차곡모아는 [restic](https://restic.net/) 으로 매일 자동 백업합니다. 한 번의 백업은 restic 스냅숏 하나이며, 그 안에 데이터베이스 덤프와 `DATA_DIR/originals/` 가 들어갑니다. `derivatives/`, `tmp/`, `orphans/`, `cache/` 는 백업하지 않습니다. 파생물은 복구한 뒤 원본에서 다시 만듭니다.

백업은 복구 키(restic 저장소 비밀번호)로 암호화됩니다. 복구 키가 없으면 백업을 열 수 없습니다.

백업과 복구는 PostgreSQL 17 서버에서만 동작합니다. 앱 이미지의 `pg_dump` 가 17 이기 때문이며, 기본 compose 는 `postgres:17-alpine` 을 사용합니다. 다른 버전이면 백업이 `backup.unsupportedPostgres` 오류로 실패하고, 앱의 다른 기능은 그대로 동작합니다.

### 백업 폴더 연결

app 서비스에 `/backup` 볼륨을 연결합니다. 앱은 이 폴더를 로컬 restic 저장소로 사용합니다. `./data` 와 다른 디스크나 NAS 공유 폴더를 권장합니다. 시놀로지 Hyper Backup 같은 도구로 이 폴더를 다시 외부로 보내도 됩니다.

```yaml
    volumes:
      - ./data:/data
      - /volume1/backup/chagokmoa:/backup
```

이 폴더는 `.env` 의 `PUID`/`PGID` 사용자가 쓸 수 있어야 합니다. 컨테이너는 기동할 때 `/backup` 이 연결되어 있으면 `./data` 와 같은 방식으로 소유권을 `PUID:PGID` 로 맞춥니다. 폴더를 만들지는 않으므로 호스트에 먼저 만들어 둡니다.

```bash
sudo mkdir -p /volume1/backup/chagokmoa
docker compose up -d
```

NAS 공유 폴더 마운트(NFS, SMB 등)에서는 소유권 변경이 거부될 수 있습니다. 이때 컨테이너 로그에 `chown /backup 실패` 경고가 남으며, 호스트나 NAS 설정에서 직접 맞춥니다.

```bash
sudo chown 1000:1000 /volume1/backup/chagokmoa   # .env 의 PUID:PGID
```

#### 시놀로지 공유 폴더

시놀로지 공유 폴더에는 기본으로 ACL(Windows 방식 접근 권한)이 걸려 있습니다. ACL 이 있는 폴더는 소유자와 일반 권한보다 ACL 이 우선하므로, 컨테이너가 기동할 때 소유권을 `PUID:PGID` 로 맞춰도 쓰기가 거부될 수 있습니다. `PUID`(예: 1000)는 DSM 사용자가 아닌 경우가 많아 DSM 화면의 공유 폴더 권한에서 고를 수도 없습니다.

증상은 다음과 같습니다.

- 백업 화면에 "백업 폴더가 연결되지 않았어요" 가 계속 표시됩니다.
- 컨테이너 안에서 `/backup` 이 `d---------` 로 보이고 파일을 만들 수 없습니다.

```bash
docker compose exec app sh -c 'ls -ldn /backup'
docker compose exec -u 1000:1000 app sh -c 'touch /backup/.t && rm /backup/.t && echo WRITE_OK'
sudo synoacltool -get /volume1/backup/chagokmoa
```

백업 전용 하위 폴더 하나만 ACL 을 지우고 일반 리눅스 권한으로 바꿉니다. 공유 폴더 전체가 아니라 앱이 쓰는 하위 폴더만 바꿉니다.

```bash
sudo synoacltool -del /volume1/backup/chagokmoa
sudo chown 1000:1000 /volume1/backup/chagokmoa   # .env 의 PUID:PGID
sudo chmod 700 /volume1/backup/chagokmoa
ls -ldn /volume1/backup/chagokmoa                # drwx------ 1000 1000, 끝에 + 가 없어야 합니다
```

`WRITE_OK` 가 나오면 앱을 다시 시작하지 않고 백업 화면을 새로 고치면 됩니다.

- 권한이 `700` 이므로 File Station 에서 일반 사용자 계정으로는 이 폴더 안을 볼 수 없습니다. 백업 내용은 restic 저장소라 직접 열어 볼 일은 거의 없습니다.
- Hyper Backup 은 시스템 권한으로 실행되므로 이 폴더를 그대로 외부로 보낼 수 있습니다.
- 다른 방법으로 `.env` 의 `PUID`/`PGID` 를 DSM 사용자의 uid, gid(`id <사용자명>` 으로 확인)로 바꿀 수 있습니다. 이 경우 다음 기동 때 `./data` 전체의 소유자도 바뀝니다.

폴더가 연결되지 않았거나 쓸 수 없으면 백업 화면에서 켤 수 없고, compose 설정 안내가 표시됩니다.

compose 의 `hostname: chagokmoa` 는 그대로 두십시오. 컨테이너를 다시 만들어도 이름이 같아서, 중단된 이전 백업이 남긴 restic 잠금을 다음 실행이 대부분 바로 정리합니다(그렇지 않은 잠금도 30분이 지나면 정리합니다).

### 켜기와 복구 키

소유자 계정으로 나 → 백업 화면에서 켭니다.

- 폴더가 비어 있으면 앱이 복구 키를 만들어 보여 줍니다. 복사하거나 텍스트 파일로 내려받아 서버 밖의 안전한 곳(비밀번호 관리자, 종이 등)에 보관한 뒤, 보관했다고 확인하면 백업이 켜집니다.
- 폴더에 이미 restic 저장소가 있으면 그 저장소의 복구 키를 입력해 이어서 백업합니다. 이전 서버의 백업을 되살리려는 경우라면 아래 [복구](#복구) 절차를 따르십시오.
- restic 저장소가 아닌 파일이 있는 폴더는 사용할 수 없습니다. 빈 폴더를 연결하십시오.

**복구 키를 잃어버리면 백업을 복구할 수 없습니다.** 복구 키는 서버 DB 에도 저장되지만 서버가 사라지면 함께 사라집니다. 반드시 서버 밖에 따로 보관하십시오. 백업 화면에서 비밀번호를 다시 입력하면 복구 키를 다시 볼 수 있습니다.

백업 화면에서 다음을 설정합니다.

- 매일 백업 시각: 인스턴스 시간대 기준, 기본 03:00
- 보관 정책: 기본 최근 7일, 주별 4개, 월별 12개
- 원격 저장소(선택, 아래 참조)

보관 정책은 매 실행 뒤 적용하며, 공간 회수(`prune`)와 무결성 확인(`check`)은 주 1회 수행합니다. 백업이 실패하면 소유자에게 앱 알림과 웹푸시로 알립니다. 같은 날 반복된 실패는 알림 하나로 묶습니다.

화면의 "지금 백업" 외에 CLI 로도 실행할 수 있습니다. 파일 소유자가 바뀌지 않도록 `PUID:PGID` 사용자로 실행합니다.

```bash
docker compose exec -u 1000:1000 app /app/chagokmoa backup
```

`-u` 없이 root 로 실행했다면 이후 앱의 백업이 권한 오류로 실패할 수 있습니다. `docker compose restart app` 으로 앱을 다시 시작하면 `DATA_DIR/cache` 와 `/backup` 의 소유권을 다시 맞춥니다. 다만 `/backup` 은 폴더 자체의 소유자가 다를 때만 안쪽까지 맞추므로, 그 뒤에도 실패하면 `/backup` 폴더를 직접 `PUID:PGID` 로 `chown -R` 합니다.

### 원격 저장소

같은 복구 키로 원격 restic 저장소에도 백업할 수 있습니다. 백업 화면에서 주소와 인증값을 입력하고 연결을 시험한 뒤 저장합니다. 원격이 비어 있으면 같은 복구 키로 저장소를 만들고, 이미 저장소가 있으면 같은 복구 키로 열리는지 확인합니다. 원격은 로컬 백업 다음에 같은 일정으로 실행됩니다.

| 종류 | 주소 예 | 인증값 |
| --- | --- | --- |
| S3 호환(AWS S3, MinIO, Wasabi, Cloudflare R2 등) | `s3:https://s3.amazonaws.com/버킷/chagokmoa` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, 필요하면 `AWS_DEFAULT_REGION`, `AWS_SESSION_TOKEN` |
| Backblaze B2 | `b2:버킷:chagokmoa` | `B2_ACCOUNT_ID`, `B2_ACCOUNT_KEY` |
| REST 서버(rest-server) | `rest:https://사용자:비밀번호@호스트:8000/chagokmoa/` | 주소 안의 사용자와 비밀번호 |
| Azure Blob Storage | `azure:컨테이너:/chagokmoa` | `AZURE_ACCOUNT_NAME` 과 `AZURE_ACCOUNT_KEY` 또는 `AZURE_ACCOUNT_SAS`, 필요하면 `AZURE_ENDPOINT_SUFFIX` |
| Google Cloud Storage | `gs:버킷:/chagokmoa` | `GOOGLE_PROJECT_ID`, `GOOGLE_APPLICATION_CREDENTIALS`(컨테이너에 연결한 서비스 계정 키 파일 경로) |

Google Cloud Storage 는 HMAC 키를 만들어 S3 호환 주소(`s3:https://storage.googleapis.com/버킷/chagokmoa`)로 쓸 수도 있습니다. sftp 와 rclone 원격은 지원하지 않습니다.

복구 키와 원격 인증값은 DB 의 설정에 저장되며, 화면, API 응답, 로그, 알림에는 표시되지 않습니다. DB 를 복구하면 백업 설정도 함께 돌아오므로, 새 서버에서 `/backup` 을 연결하면 일정이 이어집니다. 원격 저장소에서 복구했거나 디스크를 바꿔 연결한 `/backup` 이 비어 있으면, 다음 백업이 저장된 복구 키로 그 폴더에 새 저장소를 만들고 이어서 백업합니다. 이 실행은 결과가 경고(`local repository created`)로 표시되며, 새 저장소에는 그 뒤의 스냅숏만 들어갑니다. `/backup` 이 연결되지 않았거나 쓸 수 없으면 로컬 백업은 실패로 기록되고 소유자에게 알립니다.

### 부하와 메모리

- 첫 백업은 원본 전체를 읽고 저장하므로 라이브러리 크기에 따라 몇 시간이 걸릴 수 있습니다. 중간에 멈추거나 서버가 재시작되어도 다음 실행이 이미 저장한 부분부터 이어 갑니다. 이후 백업은 바뀐 부분만 저장합니다.
- restic 은 저장소 색인을 메모리에 올리므로 저장소가 클수록 메모리를 더 씁니다. 사진이 많은 저장소에서는 수백 MB 까지 쓸 수 있습니다. 메모리가 적은 NAS 에서는 업로드와 처리 작업이 적은 새벽 시각을 권장합니다.
- restic 캐시는 `DATA_DIR/cache/restic` 에 둡니다. 실행 중에만 DB 덤프를 `DATA_DIR/cache/backup/db.dump`(소유자만 읽기 가능)에 만들고 끝나면 지웁니다.

## 복구

### 새 서버에서 CLI 로 복구

복구는 앱이 멈춘 상태에서 postgres 만 띄우고 실행합니다. 앱이 DB 에 연결되어 있으면 복구는 `docker compose stop app` 하라는 안내와 함께 멈춥니다.

1. [README](../README.md) 의 설치 절차대로 `docker-compose.yml` 과 `.env` 를 준비하되, `docker compose up -d` 는 아직 실행하지 않습니다. 이미 실행했다면 `docker compose stop app` 으로 앱을 멈춥니다. 이전 `.env` 가 없으면 새로 만들어도 됩니다. 이미지는 백업한 서버와 같거나 더 새 버전을 사용합니다(더 오래된 버전은 복구한 DB 의 마이그레이션을 알지 못해 기동하지 못합니다).
2. 백업 폴더(또는 그 복사본)를 `/backup` 으로 연결합니다([백업 폴더 연결](#백업-폴더-연결)).
3. 스냅숏 목록을 확인합니다. postgres 가 함께 기동되고, 복구 키를 묻습니다(입력이 화면에 표시되지 않습니다). 목록 확인은 DB 와 파일을 바꾸지 않습니다.

   ```bash
   docker compose run --rm app restore --list
   ```

   최근 스냅숏에 이미 문제가 들어 있다면(실수로 지운 사진 등) 그 전 스냅숏의 id 를 골라 둡니다.

4. 복구합니다. `--snapshot` 을 빼면 가장 최근 스냅숏을 복구합니다.

   ```bash
   docker compose run --rm app restore --snapshot <id>
   ```

   스냅숏을 `DATA_DIR/tmp/` 아래 임시 폴더로 꺼낸 뒤, DB 덤프를 적용하고, 원본을 `DATA_DIR/originals/` 로 옮기고, 파생물 재생성 작업을 등록합니다. 임시 폴더를 거치므로 `DATA_DIR` 이 있는 디스크에 원본 전체 크기만큼 여유 공간이 필요합니다. 원격 저장소에서 꺼내는 동안에는 진행 표시가 없으며, 라이브러리 크기와 회선 속도에 따라 오래 걸릴 수 있습니다.

5. 앱을 기동합니다.

   ```bash
   docker compose up -d
   ```

   기동 후 파생물을 백그라운드에서 다시 만듭니다. 그동안 썸네일 자리에는 흐린 미리보기와 대표색만 표시됩니다. 남은 수는 나 → 백업 화면의 "파생물 다시 만들기"에서 확인할 수 있습니다.

`restore` 의 옵션은 다음과 같습니다.

| 옵션 | 설명 |
| --- | --- |
| `--list` | 스냅숏 목록(id, 시각, 파일 수, 크기, 태그)만 출력하고 끝남. DB 와 파일은 바꾸지 않으며 앱이 실행 중이어도 됨 |
| `--snapshot <id>` | 복구할 스냅숏. `--list` 에 출력된 id(앞 8자리 가능). 기본은 가장 최근 스냅숏 |
| `--repository <주소>` | 다른 저장소에서 복구. 컨테이너 안의 절대 경로 또는 원격 주소. 기본은 `/backup` |
| `--force` | 사용 중인 DB 나 파일이 있는 `originals/` 를 덮어씀. `yes` 를 입력해야 진행 |

복구는 DB 에 테이블이 없거나, 마이그레이션만 되어 사용자와 사진이 하나도 없을 때 진행합니다. 사용자나 사진이 있거나 `originals/` 에 파일이 있으면 그 내용을 출력하고 멈춥니다. 같은 서버를 이전 시점으로 되돌릴 때처럼 덮어쓰려면, 앱을 멈춘(`docker compose stop app`) 뒤 `--force` 를 붙여 다시 실행하고 `yes` 를 입력합니다. 이때 기존 DB 의 내용은 모두 지워집니다.

DB 덤프는 한 트랜잭션으로 적용합니다. 적용 중 오류가 나면 DB 는 테이블이 없는 상태로 남으므로, 원인을 고친 뒤 같은 명령을 다시 실행하면 됩니다. DB 적용 뒤 원본 이동에서 실패했다면 DB 에 이미 내용이 있으므로 `--force` 를 붙여 다시 실행합니다. 강제 종료 등으로 중단된 복구가 남긴 임시 폴더(`DATA_DIR/tmp/restore-*`)는 다음 복구가 시작할 때 지웁니다.

복구 키를 입력하지 않고 실행하려면 `RESTIC_PASSWORD` 환경 변수로 넘깁니다. 원격 저장소에서 복구할 때는 인증값을 설정 화면과 같은 이름의 환경 변수로 넘깁니다. 값을 명령줄에 직접 쓰지 않도록 셸에서 먼저 설정하고 이름만 넘기는 것을 권장합니다.

```bash
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...
docker compose run --rm -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY \
  app restore --repository s3:https://s3.amazonaws.com/버킷/chagokmoa
```

주소 안에 비밀번호가 있는 REST 서버는 `--repository` 대신 `RESTIC_REPOSITORY` 환경 변수로 넘기면 명령줄에 비밀번호가 남지 않습니다.

### 앱 없이 restic 으로 파일 꺼내기

차곡모아를 실행할 수 없어도 복구 키만 있으면 restic 으로 사진과 영상의 원본을 꺼낼 수 있습니다. 호스트에 restic 이 있다면 다음과 같이 실행합니다. 복구 키를 묻습니다.

```bash
restic -r /volume1/backup/chagokmoa snapshots
restic -r /volume1/backup/chagokmoa restore latest --target ./restored --include /data/originals
```

restic 이 없으면 차곡모아 이미지에 들어 있는 restic 을 사용합니다.

```bash
docker run --rm -it --entrypoint restic \
  -v /volume1/backup/chagokmoa:/repo -v "$PWD/restored:/restored" \
  ghcr.io/youngtaeju/chagokmoa -r /repo restore latest --target /restored --include /data/originals
```

원본은 `restored/data/originals/연/월/` 아래에 항목 id 를 이름으로 저장되어 있습니다. 원래 파일 이름, 촬영일, 아기, 이야기 등은 DB 덤프(`restored/data/cache/backup/db.dump`, `--include` 없이 복원하면 함께 나옵니다)에 있으며, `pg_restore` 로 PostgreSQL 17 에 적용하거나 `pg_restore -f dump.sql db.dump` 로 SQL 로 바꿔 볼 수 있습니다.

### 파생물 재생성

썸네일, 변환본, 포스터 등 파생물은 원본에서 다시 만들 수 있으므로 백업하지 않습니다. CLI 복구는 끝날 때 재생성 작업을 자동으로 등록합니다. 그 밖에 파생물이 없거나 손상되었을 때는 나 → 백업 화면의 "파생물 다시 만들기"를 사용하거나 CLI 로 등록합니다. 작업은 실행 중인 서버가 새 업로드보다 뒤에 처리합니다.

```bash
docker compose exec app /app/chagokmoa regenerate-derivatives --missing   # 파생물이 없는 항목만
docker compose exec app /app/chagokmoa regenerate-derivatives             # 전부
```

다시 만드는 동안에도 항목은 그대로 보이며, 새 파생물을 다 만든 뒤 교체합니다. 실패하면 기존 파생물을 유지합니다.

## 계정 복구

소유자가 비밀번호를 잃어버렸을 때(이메일 발송 기능이 없습니다) 서버에서 새 비밀번호를 발급합니다.

```bash
docker compose exec app /app/chagokmoa reset-password <username>
```

새 비밀번호가 출력되고, 해당 사용자의 기존 세션은 모두 로그아웃됩니다. 이 명령은 DB 만 사용하므로 컨테이너의 환경 변수만으로 동작합니다. 로그인한 뒤 비밀번호를 바꾸십시오.

## 로그

이미지는 JSON 형식 로그를 stderr 로 출력합니다(`LOG_FORMAT=json`).

```bash
docker compose logs -f app
```

로그 수준은 `.env` 의 `RUST_LOG`(기본 `info`)로 조절합니다. 토큰과 쿼리 문자열은 기록하지 않습니다.
