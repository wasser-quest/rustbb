-- Reactions to posts. An admin adds the kinds (a symbol, such as an emoji, and a name). A member may
-- give any number of kinds to a post, and each kind once; giving it again takes it back.
CREATE TABLE reaction_kinds (
    id       BIGSERIAL PRIMARY KEY,
    symbol   TEXT NOT NULL,
    name     TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE post_reactions (
    post_id    BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind_id    BIGINT NOT NULL REFERENCES reaction_kinds(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (post_id, user_id, kind_id)
);
CREATE INDEX post_reactions_by_post ON post_reactions (post_id);
