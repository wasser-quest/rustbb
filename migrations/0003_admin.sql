-- Admins see the report queue and may remove any post. Set by hand: UPDATE users SET is_admin = true WHERE username = '...';
ALTER TABLE users ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT false;
