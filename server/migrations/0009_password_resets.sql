CREATE TABLE password_resets (
  id         uuid PRIMARY KEY,
  user_id    uuid NOT NULL REFERENCES users(id),
  token_hash bytea NOT NULL UNIQUE,
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  used_at    timestamptz,
  revoked_at timestamptz
);
CREATE INDEX password_resets_user_id_idx ON password_resets (user_id);
