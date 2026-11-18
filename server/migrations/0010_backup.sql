-- 백업 실행 기록과 백업 실패 알림.

-- 한 번의 실행. 최근 100개 보관(실행 끝에 정리).
CREATE TABLE backup_runs (
  id          uuid PRIMARY KEY,
  started_at  timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  trigger     text NOT NULL CHECK (trigger IN ('scheduled', 'manual')),
  status      text NOT NULL CHECK (status IN ('running', 'success', 'warning', 'failed')),
  -- 진행 중 단계. 끝나면 NULL.
  phase       text CHECK (phase IN ('dump', 'local', 'remote', 'forget')),
  -- 저장소별 결과. 원격 미설정이면 remote NULL.
  local       jsonb,
  remote      jsonb,
  -- 첫 오류 문구. 비밀값 제거 후.
  error       text,
  -- prune, check 수행 여부. 주 1회 판정 기준.
  pruned      boolean NOT NULL DEFAULT false
);
CREATE INDEX backup_runs_started_at_idx ON backup_runs (started_at DESC);

-- 알림 종류 backup: 대상 id 없음, actor_id 는 받는 소유자 자신.
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('upload', 'comment', 'mention', 'story', 'milestone', 'backup'));
ALTER TABLE notifications DROP CONSTRAINT notifications_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_check CHECK (
  CASE kind
    WHEN 'upload' THEN asset_id IS NOT NULL AND story_id IS NULL AND milestone_id IS NULL
      AND asset_comment_id IS NULL AND story_comment_id IS NULL
    WHEN 'story' THEN story_id IS NOT NULL AND asset_id IS NULL AND milestone_id IS NULL
      AND asset_comment_id IS NULL AND story_comment_id IS NULL
    WHEN 'milestone' THEN milestone_id IS NOT NULL AND asset_id IS NULL AND story_id IS NULL
      AND asset_comment_id IS NULL AND story_comment_id IS NULL
    WHEN 'comment' THEN milestone_id IS NULL AND (
      (asset_id IS NOT NULL AND asset_comment_id IS NOT NULL
        AND story_id IS NULL AND story_comment_id IS NULL)
      OR (story_id IS NOT NULL AND story_comment_id IS NOT NULL
        AND asset_id IS NULL AND asset_comment_id IS NULL))
    WHEN 'mention' THEN milestone_id IS NULL AND (
      (asset_id IS NOT NULL AND asset_comment_id IS NOT NULL
        AND story_id IS NULL AND story_comment_id IS NULL)
      OR (story_id IS NOT NULL AND asset_id IS NULL AND asset_comment_id IS NULL))
    WHEN 'backup' THEN asset_id IS NULL AND story_id IS NULL AND milestone_id IS NULL
      AND asset_comment_id IS NULL AND story_comment_id IS NULL
    ELSE false
  END
);

ALTER TABLE push_mutes DROP CONSTRAINT push_mutes_kind_check;
ALTER TABLE push_mutes ADD CONSTRAINT push_mutes_kind_check
  CHECK (kind IN ('upload', 'comment', 'mention', 'story', 'milestone', 'backup'));
