-- 앱 안 알림. 받는 사람별 한 줄, 사진 올림은 같은 행동자 30분 안이면 한 줄로 묶음(count).
-- 대상 삭제(사진 완전 삭제, 이야기, 마일스톤, 댓글) 시 알림도 삭제. 댓글 알림은 댓글과 그 대상 id 를 함께 저장.

CREATE TABLE notifications (
  id               uuid PRIMARY KEY,
  user_id          uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind             text NOT NULL CHECK (kind IN ('upload', 'comment', 'mention', 'story', 'milestone')),
  actor_id         uuid NOT NULL REFERENCES users(id),
  asset_id         uuid REFERENCES assets(id) ON DELETE CASCADE,
  story_id         uuid REFERENCES stories(id) ON DELETE CASCADE,
  milestone_id     uuid REFERENCES milestones(id) ON DELETE CASCADE,
  asset_comment_id uuid REFERENCES asset_comments(id) ON DELETE CASCADE,
  story_comment_id uuid REFERENCES story_comments(id) ON DELETE CASCADE,
  count            integer NOT NULL DEFAULT 1 CHECK (count >= 1),
  created_at       timestamptz NOT NULL DEFAULT now(),
  updated_at       timestamptz NOT NULL DEFAULT now(),
  -- 종류별 대상 조합.
  CHECK (
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
      ELSE false
    END
  )
);
-- 목록 커서 순서, 안 읽은 수.
CREATE INDEX notifications_user_idx ON notifications (user_id, updated_at DESC, id DESC);
-- 대상 삭제 cascade 용. 종류별로 대부분 NULL 이라 부분 인덱스.
CREATE INDEX notifications_asset_id_idx ON notifications (asset_id) WHERE asset_id IS NOT NULL;
CREATE INDEX notifications_story_id_idx ON notifications (story_id) WHERE story_id IS NOT NULL;
CREATE INDEX notifications_milestone_id_idx ON notifications (milestone_id) WHERE milestone_id IS NOT NULL;
CREATE INDEX notifications_asset_comment_id_idx ON notifications (asset_comment_id)
  WHERE asset_comment_id IS NOT NULL;
CREATE INDEX notifications_story_comment_id_idx ON notifications (story_comment_id)
  WHERE story_comment_id IS NOT NULL;

-- 알림 화면을 마지막으로 연 시각. NULL 이면 가입 시각 기준.
ALTER TABLE users ADD COLUMN notifications_seen_at timestamptz;
