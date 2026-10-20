-- Profiles: a rank follows from the post count, an admin may set a title, and a picture is kept
-- in the object storage. A ban ends at banned_until; a permanent ban uses 31 December 9999.
ALTER TABLE users ADD COLUMN title TEXT;
ALTER TABLE users ADD COLUMN avatar_id TEXT;
ALTER TABLE users ADD COLUMN avatar_type TEXT;
ALTER TABLE users ADD COLUMN banned_until TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN ban_reason TEXT NOT NULL DEFAULT '';

-- Site switches. A missing key means off, so private messages stay off until an admin turns them on.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Private conversations between two or more members. Off unless the setting is on.
CREATE TABLE conversations (
    id         BIGSERIAL PRIMARY KEY,
    subject    TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE conversation_members (
    conversation_id BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    read_at         TIMESTAMPTZ NOT NULL DEFAULT to_timestamp(0),
    PRIMARY KEY (conversation_id, user_id)
);
CREATE INDEX conversation_members_by_user ON conversation_members (user_id);

CREATE TABLE messages (
    id              BIGSERIAL PRIMARY KEY,
    conversation_id BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body            TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX messages_by_conversation ON messages (conversation_id, id);

-- Threads that a member follows: replies in them are announced.
CREATE TABLE thread_subscriptions (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread_id  BIGINT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, thread_id)
);

-- What a member is told about: a reply in a followed thread, a mention, or a private message.
CREATE TABLE notifications (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('reply', 'mention', 'message')),
    actor_id        BIGINT REFERENCES users(id) ON DELETE CASCADE,
    thread_id       BIGINT REFERENCES threads(id) ON DELETE CASCADE,
    post_id         BIGINT REFERENCES posts(id) ON DELETE CASCADE,
    conversation_id BIGINT REFERENCES conversations(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    read_at         TIMESTAMPTZ
);
CREATE INDEX notifications_unread ON notifications (user_id) WHERE read_at IS NULL;

-- A poll belongs to one thread: one question, options, and one vote per member (changeable).
CREATE TABLE polls (
    id        BIGSERIAL PRIMARY KEY,
    thread_id BIGINT NOT NULL UNIQUE REFERENCES threads(id) ON DELETE CASCADE,
    question  TEXT NOT NULL
);

CREATE TABLE poll_options (
    id       BIGSERIAL PRIMARY KEY,
    poll_id  BIGINT NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    text     TEXT NOT NULL,
    position INTEGER NOT NULL
);
CREATE INDEX poll_options_by_poll ON poll_options (poll_id, position);

CREATE TABLE poll_votes (
    poll_id   BIGINT NOT NULL REFERENCES polls(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    option_id BIGINT NOT NULL REFERENCES poll_options(id) ON DELETE CASCADE,
    PRIMARY KEY (poll_id, user_id)
);

-- A link to set a new password. Only a hash of the token is stored; it works once.
CREATE TABLE password_resets (
    token_hash TEXT PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
