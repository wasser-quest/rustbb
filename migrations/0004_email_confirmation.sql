-- A new account confirms its e-mail address before it can log in. Accounts from before this
-- migration have no address; they count as confirmed from the day they were created.
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN email_confirmed_at TIMESTAMPTZ;
UPDATE users SET email_confirmed_at = created_at;

-- Only a confirmed address is unique. An unconfirmed sign-up must not block the real owner.
CREATE UNIQUE INDEX users_confirmed_email ON users (lower(email)) WHERE email_confirmed_at IS NOT NULL;

-- Only a hash of the link's token is stored, as for sessions.
CREATE TABLE email_confirmations (
    token_hash TEXT PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
