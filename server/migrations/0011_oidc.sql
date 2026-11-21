-- OIDC 로그인 연결과 비밀번호 없는 계정.

-- NULL: 차곡모아 비밀번호 없음(외부 IdP 로만 로그인).
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- IdP 계정 연결. issuer 함께 저장(IdP 교체 시 다른 IdP 의 같은 sub 와 섞임 방지).
ALTER TABLE users
  ADD COLUMN oidc_issuer  text,
  ADD COLUMN oidc_subject text,
  ADD CONSTRAINT users_oidc_link_check
    CHECK ((oidc_issuer IS NULL) = (oidc_subject IS NULL));

CREATE UNIQUE INDEX users_oidc_link_idx
  ON users (oidc_issuer, oidc_subject) WHERE oidc_subject IS NOT NULL;
