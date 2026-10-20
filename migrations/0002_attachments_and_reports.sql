-- Attachments live in S3-compatible storage; the database only keeps their metadata.
CREATE TABLE attachments (
    id           TEXT PRIMARY KEY,            -- random, public in the URL /files/<id>
    post_id      BIGINT REFERENCES posts(id) ON DELETE CASCADE,
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content_type TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX attachments_by_post ON attachments (post_id);

-- Posts can be removed by their author; the thread row stays until its last post goes.
ALTER TABLE posts ADD COLUMN deleted_at TIMESTAMPTZ;

CREATE TABLE reports (
    id         BIGSERIAL PRIMARY KEY,
    post_id    BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason     TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (post_id, user_id)
);

-- One-time answers to the anti-bot question shown when registering.
CREATE TABLE captchas (
    id          TEXT PRIMARY KEY,
    answer_hash TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX captchas_expiry ON captchas (expires_at);
