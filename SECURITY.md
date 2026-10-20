# Security

## What the forum does

- **Output**: every value is escaped by maud. Post text is escaped first; the only markup added is a link for a plain `http://` or `https://` URL (`rel="nofollow ugc noopener noreferrer"`). `javascript:`, `data:` and anything with quotes or angle brackets stays plain text.
- **No scripts**: pages send `Content-Security-Policy: default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'`, plus `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`.
- **Sessions**: a random 256-bit token in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` unless `FORUM_SECURE_COOKIES=false`). Only the SHA-256 of the token is stored; sessions last 30 days.
- **CSRF**: every state-changing form carries a per-session token that is checked on the server.
- **Passwords**: Argon2id with a random salt. Unknown usernames are checked against a dummy hash to keep the response time the same.
- **Anti-bot**: a one-time arithmetic question at registration. It expires after 10 minutes and is deleted after one answer.
- **Rate limits**: sign-ups 5 per address per hour, logins 20 per address per 15 minutes, posts 20 per account per 10 minutes, reports 10 per account per hour.
- **Mail quota**: every outgoing message (confirmation, password reset) passes a quota that also holds with `FORUM_RATE_LIMIT=off`: 3 messages per recipient per hour and 6 per day, and 100 messages per hour and 300 per day for the whole forum. Nobody can use the forum to flood a mailbox.
- **Attachments**: only PNG, JPEG, GIF and WebP, chosen by their first bytes (not the file name or the browser's type). At most 5 MB per file and 4 per post. SVG and HTML are refused. Files are stored in S3-compatible storage under random 128-bit ids, and served from `/files/<id>` with `nosniff` and a `sandbox` policy. They are never served from the bucket directly.
- **Deletion**: an author can remove their own post; its files are deleted from storage. Moderators (`is_admin`) can remove any post from the report queue.
- **Logs**: database and storage errors are written to the server log; visitors see a plain message.

## Known limits

- Rate limits and the proxy-address handling are in memory and per process. Run one instance, or move the counters into the database before scaling out.
- The anti-bot question stops simple scripts, not a determined attacker or a human farm. Add email verification or a stronger challenge before opening registration widely.
- Images are not re-encoded, so metadata (for example EXIF location) is kept. Re-encoding would remove it and is the next step.
- No malware scanning for uploads.
- Accounts have no email address, so there is no password reset yet.
- Trust in `X-Forwarded-For` depends on `FORUM_TRUST_PROXY`; enable it only behind a proxy that sets the header.

## Reporting a problem

Send details to the maintainers privately before publishing them.
