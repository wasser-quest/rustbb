# rustbb

A bulletin board in pure Rust. Boards, threads and replies, with images, polls and reactions. The
pages are server-rendered HTML and run no JavaScript, so there is no script to attack in a visitor's
browser.

## Features

- **Boards, threads and replies**, with paging and search by thread title.
- **Images** (PNG, JPEG, GIF, WebP) in posts and replies, stored in S3-compatible object storage.
  Files are checked by their first bytes, not by name, and SVG and HTML are refused.
- **Post markup** without scripts: `[b]`, `[i]`, `[code]`, `[spoiler]`, `[quote=name]` and
  `[embed title="…" url="…" color="blue"]`, with `[tabs]` and `[tab=Title]` inside them. The help page
  at `/help` lists all tags.
- **Polls** in new threads, one vote per member, changeable.
- **Reactions** to posts. Admins add the kinds (a symbol and a name) under *Reactions*.
- **Following and notifications**: follow a thread to hear about its replies; mentions with `@name`.
- **Profiles** with a rank from the number of posts, an optional title and a picture.
- **Moderation**: admins remove posts, ban members for a number of days or permanently, and set titles.
- **Private messages**, off until an admin switches them on under *Site*.
- **Accounts**: registration with an e-mail confirmation, password reset by e-mail, an anti-bot
  question, and account deletion (posts stay, shown as "[deleted]"). The first account ever made becomes the admin.
- **Privacy**: metadata such as the location in a picture (EXIF) is removed on upload.
- **Search engine friendly**: descriptive thread addresses, canonical links, `robots.txt`, a
  sitemap, and `noindex` on the private pages.

## Requirements

- Rust 1.90 or newer (edition 2024)
- PostgreSQL (tested with version 17; the migrations run on start)
- S3-compatible object storage for attachments
- A mail server for confirmations and resets: `sendmail` (for example msmtp with an SMTP relay), or a
  file for testing

## Run it

    export FORUM_DATABASE_URL=postgres://forum:secret@localhost/forum
    export FORUM_S3_BUCKET=my-bucket
    export FORUM_S3_REGION=eu-central-1
    export FORUM_S3_ENDPOINT=https://s3.example.org     # leave out for AWS
    export AWS_ACCESS_KEY_ID=…
    export AWS_SECRET_ACCESS_KEY=…
    export FORUM_MAIL_FROM=noreply@example.org
    export FORUM_PUBLIC_URL=https://forum.example.org
    export FORUM_SECURE_COOKIES=true                    # false only for local http
    cargo run --release

The forum listens on `FORUM_BIND` (default `127.0.0.1:8080`). Behind a reverse proxy, set
`FORUM_TRUST_PROXY=true` so the rate limits see the real client address.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `FORUM_DATABASE_URL` | required | PostgreSQL connection |
| `FORUM_BIND` | `127.0.0.1:8080` | address to listen on |
| `FORUM_PUBLIC_URL` | `http://127.0.0.1:8080` | the address the forum is reached at (links in mails) |
| `FORUM_SECURE_COOKIES` | `true` | cookies only over https |
| `FORUM_TRUST_PROXY` | `false` | read the client address from `X-Forwarded-For` |
| `FORUM_RATE_LIMIT` | on | `off` leaves rate limiting to a load balancer (see `deploy/`) |
| `FORUM_S3_BUCKET` | required | bucket for attachments |
| `FORUM_S3_REGION` | `us-east-1` | region of the bucket |
| `FORUM_S3_ENDPOINT` | AWS | endpoint of an S3-compatible service (https in production) |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | required | storage credentials |
| `FORUM_MAIL_MODE` | `sendmail` | `sendmail`, or `file` (writes the mail to `FORUM_MAIL_FILE`, for tests) |
| `FORUM_MAIL_FROM` | required for `sendmail` | sender address |
| `FORUM_SENDMAIL` | `/usr/sbin/sendmail` | the sendmail program |
| `FORUM_MAIL_FILE` | | file for `file` mode |
| `FORUM_UPLOAD_USER_MB` | see `src/quota.rs` | attachments one member may keep |
| `FORUM_UPLOAD_USER_FILES_PER_DAY` | see `src/quota.rs` | new files one member may add per day |
| `FORUM_UPLOAD_TOTAL_GB` | see `src/quota.rs` | attachments of all members together |

## Tests

    cargo test                      # unit tests

The end-to-end test runs against a running forum with `FORUM_MAIL_MODE=file`, and needs a Docker
container with the database (its name in `DB_CONTAINER`) for the admin steps:

    FORUM_URL=http://127.0.0.1:8080 DB_CONTAINER=forum-test-db FORUM_MAIL_FILE=/tmp/forum-mail.txt \
      python3 tests/e2e.py

## Deployment

- `Dockerfile`: builds the release binary and runs it on Debian slim, with msmtp as the local mail
  relay. The relay's settings go into `/etc/msmtprc` at run time, not into the image
  (see `deploy/msmtprc.example`).
- `deploy/nginx-rate-limit.conf.example`: rate limits for sign-up, login and confirmation at the
  reverse proxy.

## Security

See [SECURITY.md](SECURITY.md) for what the forum does, and what it does not do yet.

## License

MIT, see [LICENSE](LICENSE).
