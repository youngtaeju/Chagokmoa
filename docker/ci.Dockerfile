# CI server 잡 전용 빌드 이미지. 잡 실행 중 apt 미러 접속 없게 의존성 미리 설치.
# 런타임 이미지(docker/Dockerfile)와 같은 Debian trixie 계열. 목록 바뀌면 내용 해시 태그도 바뀜.
FROM rust:1.99-trixie

# git: OpenAPI 신선도 검사의 git diff. postgresql-client-17: 백업 덤프 왕복 테스트.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config zlib1g-dev libheif-dev \
        libheif-plugin-libde265 libheif-plugin-aomenc libheif-plugin-dav1d \
        ffmpeg restic postgresql-client-17 git \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy \
    && cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features postgres,rustls \
    && rm -rf /usr/local/cargo/registry
