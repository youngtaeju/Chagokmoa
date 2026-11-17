# syntax=docker/dockerfile:1
# 빌드 컨텍스트: 레포 루트. docker build -f docker/Dockerfile .

FROM node:24-trixie-slim AS web
# pnpm 버전은 package.json 의 packageManager 기준.
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN --mount=type=cache,id=chagokmoa-pnpm,target=/pnpm-store \
    pnpm install --frozen-lockfile --store-dir /pnpm-store
COPY web/ ./
RUN pnpm build

FROM rust:1.99-trixie AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config zlib1g-dev libheif-dev \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src/server

# 의존성 목록(recipe.json)만 추출. Cargo.toml, Cargo.lock 이 같으면 같은 결과.
FROM chef AS planner
COPY server/ ./
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS server
# sqlx 매크로는 커밋된 .sqlx/ 로 검사. 빌드에 DB 불필요.
ENV SQLX_OFFLINE=true
# 의존성 빌드를 일반 레이어로. 캐시 마운트는 CI 레이어 캐시(type=gha)에 안 남아 사용 안 함.
COPY --from=planner /src/server/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY web/src/locales/ /src/web/src/locales/
COPY server/ ./
RUN cargo build --release --locked \
    && cp target/release/chagokmoa /chagokmoa

FROM debian:trixie-slim AS runtime
# --no-install-recommends: libheif1 의 Recommends 인 x265(GPL) 플러그인 제외.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl ffmpeg libheif1 \
        libheif-plugin-libde265 libheif-plugin-dav1d libheif-plugin-aomenc \
    && rm -rf /var/lib/apt/lists/*
COPY --from=server /chagokmoa /app/chagokmoa
COPY --from=web /src/web/dist /app/web
COPY --chmod=755 docker/entrypoint.sh /app/entrypoint.sh
ENV DATA_DIR=/data \
    WEB_DIR=/app/web \
    PORT=8080 \
    LOG_FORMAT=json \
    AUTO_MIGRATE=true \
    PUID=1000 \
    PGID=1000
WORKDIR /app
EXPOSE 8080
ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["serve"]
