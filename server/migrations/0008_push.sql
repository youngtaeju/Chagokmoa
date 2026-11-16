-- 웹푸시 구독, 종류별 끔, 알림 발송 장수.

CREATE TABLE push_subscriptions (
    session_id uuid PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    endpoint text NOT NULL UNIQUE,
    p256dh text NOT NULL,
    auth text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- 끈 종류만 행.
CREATE TABLE push_mutes (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('upload', 'comment', 'mention', 'story', 'milestone')),
    PRIMARY KEY (user_id, kind)
);

-- 기존 줄은 이미 발송된 것으로 간주. 배포 직후 옛 묶음 일괄 발송 방지.
ALTER TABLE notifications ADD COLUMN pushed_count integer NOT NULL DEFAULT 0;
UPDATE notifications SET pushed_count = count;
