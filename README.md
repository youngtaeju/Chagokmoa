<p align="center">
  <img src="docs/assets/banner.ko.png" alt="차곡모아: 우리 아이의 하루하루를 차곡차곡 모아 두는 곳" width="100%">
</p>

<p align="center">
  <b>한국어</b> | <a href="README.en.md">English</a>
</p>

<p align="center">
  <a href="https://github.com/youngtaeju/Chagokmoa/releases/latest"><img src="https://img.shields.io/github/v/release/youngtaeju/Chagokmoa?label=release" alt="최신 릴리스"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/youngtaeju/Chagokmoa" alt="라이선스"></a>
</p>

## 소개

가족이 함께 쓰는 셀프호스팅 아기 포토 저널입니다. 아이의 사진과 영상, 자라는 기록을 집에서 직접 운영하는 서버 한 대에 모아 둡니다. 서버 하나가 가족 하나를 담당하며, 외부 클라우드, 광고, 추적, 구독 없이 동작합니다. 사진과 영상의 원본은 올린 그대로 보존합니다.

## 기능

**사진과 영상**

- 여러 장을 한 번에 올리고, 끊겨도 이어서 올립니다. 같은 파일은 다시 올라가지 않습니다.
- HEIC 사진과 휴대폰 영상을 지원합니다. 원본은 바이트 그대로 보관하고, 화면에는 썸네일과 웹에서 재생되는 변환본을 씁니다.
- 촬영일 순 타임라인, 전체 화면 뷰어, 원본 내려받기를 제공합니다.
- 지운 사진은 휴지통을 거치므로 실수로 잃지 않습니다.

**가족**

- 가족은 초대 링크로만 합류합니다. 역할은 관리자, 보호자(사진 올리기와 아기 정보 관리), 가족(보기와 반응) 세 가지입니다.
- 하트, 댓글, 멘션, 즐겨찾기로 함께 봅니다.
- 새 사진, 댓글, 멘션, 이야기, 마일스톤을 웹푸시 알림으로 받습니다.

**기록**

- 아기 프로필과 생후 일수, 개월 수를 보여 줍니다. 아기가 여럿이어도 됩니다.
- 이야기(글과 사진), 키와 몸무게, 뒤집기와 혼자 걷기와 첫 이 같은 마일스톤을 기록합니다.
- 성장 차트에서 WHO 또래 성장 곡선과 함께 봅니다.

**둘러보기**

- 달력으로 날짜별 사진을 찾고, 1년 전 오늘이나 몇 개월 전 오늘의 추억을 다시 봅니다.
- 앨범으로 사진을 묶습니다.

**운영**

- 브라우저와 홈 화면에 설치하는 앱(PWA)으로 씁니다. 다크 모드와 한국어, 영어를 지원합니다.
- Docker 이미지 하나와 PostgreSQL 하나로 홈서버나 NAS 에서 실행합니다.
- restic 기반 암호화 백업(로컬 폴더와 원격 저장소)과 복구를 앱 안에서 설정합니다.
- 선택 기능: SMTP 메일(초대, 비밀번호 찾기), OIDC 로그인(Authentik, Authelia, Keycloak, Google 등).

## 브라우저와 홈 화면 앱

설치한 서버 주소를 브라우저로 열면 바로 쓸 수 있습니다. 휴대폰이나 PC 에 앱으로 설치(PWA)하면 주소창 없이 전체 화면으로 열리고 푸시 알림을 받을 수 있습니다. 앱 스토어는 거치지 않습니다.

- iPhone, iPad: Safari 의 공유 버튼에서 "홈 화면에 추가"
- Android: Chrome 메뉴에서 앱 설치(또는 홈 화면에 추가)
- PC: Chrome, Edge 주소창의 설치 아이콘

푸시 알림은 설정 → 알림 설정에서 기기마다 켭니다. iPhone, iPad 는 홈 화면에 추가한 앱에서만 푸시 알림을 받을 수 있습니다. 앱 설치와 푸시 알림은 https(또는 localhost) 주소에서만 동작합니다.

### 지원 브라우저

- 최근 2년 이내 버전의 Chrome, Edge, Firefox, Safari
- iOS 16.4 이상의 Safari (홈 화면에 추가한 앱 포함)
- Android 의 Chrome

`Intl.supportedValuesOf` 를 지원하지 않는 브라우저에서는 시간대 선택에 현재 값만 표시됩니다.

## 설치

Docker 와 Docker Compose 가 있는 서버(홈서버, NAS 등)에서 실행합니다. 이미지는 `ghcr.io/youngtaeju/chagokmoa` 이며 `linux/amd64` 를 지원합니다. 외부 의존은 PostgreSQL 하나이고 compose 에 포함되어 있습니다.

1. `compose/` 디렉터리의 `docker-compose.yml` 과 `.env.example` 을 서버의 작업 디렉터리에 복사하고, `.env.example` 을 `.env` 로 바꿔 값을 채웁니다.

   - `SECRET_KEY`: `openssl rand -hex 32` 로 만든 64자 16진수입니다.
   - `POSTGRES_PASSWORD`: DB 비밀번호입니다. `DATABASE_URL` 안의 비밀번호와 같은 값이어야 합니다. 영숫자를 권장하며, 특수문자는 `DATABASE_URL` 에서 URL 인코딩이 필요합니다.
   - `PUBLIC_URL`: 브라우저로 접속하는 주소입니다. 리버스 프록시 뒤라면 그 https 주소입니다.
   - `PUID`, `PGID`: `./data` 디렉터리를 소유할 호스트 사용자입니다. 호스트에서 `id -u`, `id -g` 로 확인합니다.

2. 실행합니다.

   ```bash
   docker compose up -d
   ```

   데이터베이스 마이그레이션은 기동할 때 자동으로 적용됩니다. 원본과 파생물은 `./data`, 데이터베이스는 `./postgres` 에 저장됩니다.

3. 브라우저에서 `PUBLIC_URL` 주소(기본 `http://localhost:8080`)를 열고 첫 가입을 합니다. 첫 번째로 가입한 계정이 관리자(owner)가 되며, 이후 가족은 관리자가 만든 초대 링크로 가입합니다.

compose 의 `ports` 는 호스트의 모든 인터페이스에 포트를 엽니다. 같은 호스트에서 리버스 프록시를 운영한다면 `127.0.0.1` 에만 바인딩합니다.

```yaml
    ports:
      - "127.0.0.1:${PORT:-8080}:8080"
```

리버스 프록시, https, 백업, 메일, 로그인(OIDC) 등 운영 안내는 [운영 문서](docs/operations.md)를 참고하십시오.

### 업그레이드

`.env` 의 `CHAGOKMOA_VERSION` 을 새 버전으로 바꾼 뒤 다음을 실행합니다. 기본값 `latest` 를 쓰는 경우 값 변경 없이 실행합니다.

```bash
docker compose pull && docker compose up -d
```

새 버전의 마이그레이션은 기동할 때 자동으로 적용됩니다. 업그레이드 전에 백업을 권장합니다([백업](docs/operations.md#백업)).

## 라이선스

Copyright (C) 2026 youngtaeju

AGPL-3.0-only. [LICENSE](LICENSE) 참조.

Docker 이미지에는 서드파티 패키지가 함께 들어 있으며 각각의 라이선스를 따릅니다. 예를 들어 Debian 의 ffmpeg 는 GPL 구성요소(x264, x265 등)를 링크하고, 이미지에는 libheif 등도 포함됩니다. 해당 패키지의 소스는 Debian 에서 받을 수 있습니다.

성장 차트의 또래 성장 곡선은 WHO Child Growth Standards 표를 월 단위로 줄여 내장했습니다. 표 자체에는 라이선스 명시가 없어 WHO 출판물 기준인 CC BY-NC-SA 3.0 IGO(비상업)로 간주합니다. 출처와 이용 조건은 [web/src/growth/who/NOTICE.md](web/src/growth/who/NOTICE.md) 에 있습니다.
