#!/usr/bin/env bash
# 백업과 복구 왕복. 빈 스택에 사진 업로드 → CLI 백업 → DB 와 데이터 삭제 → 새 스택에서 CLI 복구
# → 사진 목록, 원본 해시 일치와 파생물 재생성 확인. 이미지 안의 pg_dump, pg_restore(17) 와 restic 사용.
# 사용: docker/backup-smoke.sh [이미지 태그]   (기본 chagokmoa:dev, 호스트 포트 SMOKE_PORT 기본 18091)
set -euo pipefail

IMAGE="${1:-chagokmoa:dev}"
PORT="${SMOKE_PORT:-18091}"
PROJECT=chagokmoa-backup-smoke
PUID=1234
WORK="$(mktemp -d)"
BASE="http://localhost:$PORT"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JAR="$WORK/cookies"
PASSWORD=backup-smoke-123
# shellcheck source=docker/stack.sh
. "$(dirname "$0")/stack.sh"

cleanup() {
    local status=$?
    if [ "$status" -ne 0 ] && [ -f "$WORK/stack.override.yml" ]; then
        compose logs --no-color app >&2 || true
    fi
    # 백업 폴더는 PUID 소유. 컨테이너 안에서 삭제.
    docker run --rm --entrypoint rm -v "$WORK:/w" "$IMAGE" -rf /w/backup >/dev/null 2>&1 || true
    stack_down
    rm -rf "$WORK"
    if [ "$status" -eq 0 ]; then echo "BACKUP SMOKE PASS"; else echo "BACKUP SMOKE FAIL" >&2; fi
    exit "$status"
}
trap cleanup EXIT

pass() { echo "PASS: $*"; }

api() {
    curl -fsS -b "$JAR" -c "$JAR" -H 'X-Requested-With: Chagokmoa' "$@"
}

login() {
    rm -f "$JAR"
    api -o /dev/null -H 'Content-Type: application/json' \
        -d "{\"username\":\"mom\",\"password\":\"$PASSWORD\"}" "$BASE/api/auth/login"
}

# tus 생성, 한 번에 PATCH, finalize. 자산 id 출력.
upload() {
    local file=$1 name size location id
    name="$(basename "$file")"
    size="$(stat -c %s "$file")"
    location="$(api -D - -o /dev/null -X POST "$BASE/api/uploads" \
        -H 'Tus-Resumable: 1.0.0' -H "Upload-Length: $size" \
        -H "Upload-Metadata: filename $(printf %s "$name" | base64 -w0)" |
        tr -d '\r' | awk 'tolower($1) == "location:" { print $2 }')"
    [ -n "$location" ] || fail "$name 업로드 생성 실패"
    api -o /dev/null -X PATCH "$BASE$location" \
        -H 'Tus-Resumable: 1.0.0' -H 'Upload-Offset: 0' \
        -H 'Content-Type: application/offset+octet-stream' --data-binary "@$file"
    id="$(api -H 'Content-Type: application/json' -d '{}' "$BASE$location/finalize" | jq -r .id)"
    [ -n "$id" ] && [ "$id" != null ] || fail "$name finalize 실패"
    echo "$id"
}

# 휴지통 제외 사진 목록. "id status" 줄, id 순.
assets() {
    api "$BASE/api/assets?limit=100" | jq -r '.items[] | "\(.id) \(.status)"' | sort
}

wait_ready() {
    local deadline=$((SECONDS + 120))
    until [ -n "$(assets)" ] && ! assets | grep -qv ' ready$'; do
        if [ "$SECONDS" -ge "$deadline" ]; then fail "처리 120초 안에 끝나지 않음: $(assets)"; fi
        sleep 2
    done
}

originals_hashes() {
    compose exec -T app sh -c 'cd /data/originals && find . -type f -exec sha256sum {} + | sort'
}

sql() {
    compose exec -T postgres psql -U chagokmoa -d chagokmoa -tAc "$1"
}

# 앱 컨테이너 안 restic. 키는 환경 변수로만.
restic_in_app() {
    compose exec -T -u "$PUID:$PUID" -e RESTIC_PASSWORD="$KEY" -e RESTIC_REPOSITORY=/backup app \
        restic --no-cache "$@"
}

# --- 1. 빈 스택, /backup 연결 ---

stack_write
cat >"$WORK/stack.override.yml" <<EOF
services:
  app:
    image: $IMAGE
    volumes:
      - ./backup:/backup
EOF
# root 소유 백업 폴더. entrypoint 가 PUID 소유로 보정해야 함.
docker run --rm --entrypoint mkdir -v "$WORK:/w" "$IMAGE" /w/backup
owner="$(stat -c %u "$WORK/backup")"
[ "$owner" = 0 ] || fail "백업 폴더 준비 소유자 $owner (기대 0)"

compose up -d --quiet-pull
wait_healthy
owner="$(compose exec -T app stat -c %u:%g /backup)"
[ "$owner" = "$PUID:$PUID" ] || fail "/backup 소유자 $owner (기대 $PUID:$PUID)"
pass "entrypoint 가 /backup 소유권 보정"
api -o /dev/null -H 'Content-Type: application/json' \
    -d "{\"username\":\"mom\",\"password\":\"$PASSWORD\",\"display_name\":\"mom\",\"instance_name\":\"smoke\"}" \
    "$BASE/api/auth/signup"
for f in web/e2e/files/orient6.jpg web/e2e/files/rotated.heic web/e2e/files/clip-h264.mp4; do
    upload "$ROOT/$f" >/dev/null
done
wait_ready
before_assets="$(assets)"
[ "$(wc -l <<<"$before_assets")" -eq 3 ] || fail "자산 3개 아님: $before_assets"
before_hashes="$(originals_hashes)"
pass "업로드 3개 처리 완료"

# --- 2. 화면 흐름으로 백업 켜기, CLI 백업 ---

KEY="$(api -X POST "$BASE/api/backup/key" | jq -r .key)"
[ -n "$KEY" ] && [ "$KEY" != null ] || fail "복구 키 발급 실패"
api -o /dev/null -H 'Content-Type: application/json' \
    -d "{\"key\":\"$KEY\",\"confirmed\":true}" "$BASE/api/backup/enable"
pass "백업 켜기(restic init)"

# pg_dump 가 미리 만든 0600 파일 권한을 유지하는지(덤프에 복구 키와 인증값 포함).
mode="$(compose exec -T -u "$PUID:$PUID" app sh -c '
    set -e
    umask 022
    f=$(mktemp)
    chmod 600 "$f"
    PGPASSWORD=stack pg_dump --no-password --format=custom --file="$f" \
        --dbname=postgres://chagokmoa@postgres:5432/chagokmoa
    stat -c %a "$f"
    rm -f "$f"
')"
[ "$mode" = 600 ] || fail "pg_dump 후 덤프 파일 권한 $mode (기대 600)"
pass "pg_dump 가 0600 유지"

# 예정 시각(03:00)이 지난 날이면 켜자마자 자동 백업이 돌 수 있음. 자동은 끄고, 그 실행과 겹치면 재시도.
api -o /dev/null -X PUT -H 'Content-Type: application/json' -d '{"enabled":false}' "$BASE/api/backup"
for _ in $(seq 30); do
    set +e
    out="$(compose exec -T -u "$PUID:$PUID" app /app/chagokmoa backup 2>&1)"
    code=$?
    set -e
    grep -q 'backup.running' <<<"$out" || break
    sleep 5
done
[ "$code" -eq 0 ] || fail "CLI 백업 종료 코드 $code: $out"
grep -q '^status Success' <<<"$out" || fail "CLI 백업 결과: $out"
grep -qF "$KEY" <<<"$out" && fail "CLI 백업 출력에 복구 키"
pass "CLI 백업"

# 실제 백업 실행이 만든 덤프가 스냅숏 안에서도 0600(0o600 = 384).
dump_node="$(restic_in_app ls --json latest | grep '"name":"db.dump"')" || fail "스냅숏에 db.dump 없음"
grep -q '"mode":384[,}]' <<<"$dump_node" || fail "스냅숏 db.dump 권한: $dump_node"
pass "스냅숏의 덤프 0600"
SNAPSHOT="$(restic_in_app snapshots --json | jq -r '.[-1].short_id')"

# --- 3. DB 와 데이터 삭제(백업 폴더만 남김) ---

compose down -v --remove-orphans >/dev/null 2>&1
docker run --rm --entrypoint rm -v "$WORK:/w" "$IMAGE" -rf /w/data /w/postgres
[ -f "$WORK/backup/config" ] || fail "백업 폴더 유실"
pass "DB 와 데이터 삭제"

# --- 4. 새 스택에서 복구(마이그레이션만 된 빈 DB, 앱은 기동 전) ---

compose run --rm --no-TTY app migrate >/dev/null 2>&1 || fail "migrate 실패"
out="$(RESTIC_PASSWORD="$KEY" compose run --rm --no-TTY -e RESTIC_PASSWORD app restore --list 2>&1)" ||
    fail "restore --list 실패: $out"
grep -q "^  $SNAPSHOT " <<<"$out" || fail "--list 에 스냅숏 $SNAPSHOT 없음: $out"
[ "$(sql 'SELECT count(*) FROM users')" = 0 ] || fail "--list 가 DB 를 바꿈"
pass "restore --list 는 변경 없음"
# 강제 종료된 이전 복구의 잔재. 복구가 시작할 때 지워야 함(아래 기동 후 확인).
docker run --rm --entrypoint sh -v "$WORK/data:/data" "$IMAGE" -c \
    "mkdir -p /data/tmp/restore-interrupted && echo x >/data/tmp/restore-interrupted/db.dump && chown -R $PUID:$PUID /data/tmp"
set +e
out="$(RESTIC_PASSWORD="$KEY" compose run --rm --no-TTY -e RESTIC_PASSWORD app restore --snapshot "$SNAPSHOT" 2>&1)"
code=$?
set -e
echo "$out" | grep -v '^{' || true
[ "$code" -eq 0 ] || fail "restore 종료 코드 $code: $out"
grep -qF "$KEY" <<<"$out" && fail "restore 출력에 복구 키"
grep -q '^removed 1 temp dirs left by an interrupted restore' <<<"$out" || fail "이전 임시 폴더 정리 출력 없음: $out"
grep -q '^restored 3 originals' <<<"$out" || fail "restore 원본 수: $out"
grep -q '^queued 3 derivative regenerations' <<<"$out" || fail "restore 재생성 작업 수: $out"
pass "CLI 복구"

regen="$(sql "SELECT count(*) FROM jobs WHERE kind = 'regenerate_derivatives' AND status = 'queued'")"
[ "$regen" = 3 ] || fail "재생성 작업 $regen (기대 3)"
pass "재생성 작업 3개 대기"

# 두 번째 복구는 사용 중 DB 라 거부.
set +e
out="$(RESTIC_PASSWORD="$KEY" compose run --rm --no-TTY -e RESTIC_PASSWORD app restore 2>&1)"
code=$?
set -e
[ "$code" -ne 0 ] || fail "사용 중 DB 에 복구가 진행됨"
grep -q 'database has 1 users and 3 photos and videos' <<<"$out" || fail "거부 출력: $out"
pass "사용 중 DB 복구 거부"

# --- 5. 기동 후 확인 ---

compose up -d
wait_healthy
# 앱 실행 중 복구는 DB 를 건드리기 전에 거부.
set +e
out="$(RESTIC_PASSWORD="$KEY" compose run --rm --no-TTY -e RESTIC_PASSWORD app restore --force 2>&1 </dev/null)"
code=$?
set -e
[ "$code" -ne 0 ] || fail "앱 실행 중 복구가 진행됨"
grep -q 'docker compose stop app' <<<"$out" || fail "앱 실행 중 거부 출력: $out"
pass "앱 실행 중 복구 거부"
login
[ "$(assets)" = "$before_assets" ] || fail "자산 목록 다름: $(assets)"
[ "$(originals_hashes)" = "$before_hashes" ] || fail "원본 해시 다름"
for path in /data/originals /data/tmp; do
    owner="$(compose exec -T app stat -c %u "$path")"
    [ "$owner" = "$PUID" ] || fail "$path 소유자 $owner (기대 $PUID)"
done
leftover="$(compose exec -T app sh -c 'ls -A /data/tmp | grep "^restore" || true')"
[ -z "$leftover" ] || fail "복구 임시 폴더 남음: $leftover"
pass "자산 목록, 원본 해시, 소유자 일치"

deadline=$((SECONDS + 180))
for id in $(awk '{ print $1 }' <<<"$before_assets"); do
    until api -o /dev/null "$BASE/api/assets/$id/thumb.webp" 2>/dev/null; do
        if [ "$SECONDS" -ge "$deadline" ]; then fail "$id 썸네일 180초 안에 200 아님"; fi
        sleep 2
    done
done
pass "재생성 후 썸네일 200"

status="$(api "$BASE/api/backup")"
jq -e '.mounted and .key_set and .repo_state == "restic" and (.enabled | not)' <<<"$status" >/dev/null ||
    fail "복구 후 백업 상태: $status"
pass "복구 후 백업 설정 유지"
