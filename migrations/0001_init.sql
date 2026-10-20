CREATE TABLE users (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only a hash of the session token is stored. The CSRF token is per session.
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE boards (
    id          BIGSERIAL PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE threads (
    id           BIGSERIAL PRIMARY KEY,
    board_id     BIGINT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_post_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX threads_by_board ON threads (board_id, last_post_at DESC);

CREATE TABLE posts (
    id         BIGSERIAL PRIMARY KEY,
    thread_id  BIGINT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX posts_by_thread ON posts (thread_id, id);

INSERT INTO boards (slug, title, description, position) VALUES
    ('general',  'General',  'Talk about anything', 0),
    ('learning', 'Learning', 'The method of loci and memory palaces', 1);
