#!/usr/bin/env python3
"""End-to-end check of the forum against a running server (FORUM_URL, default http://127.0.0.1:8090).

It needs a database user that can run SQL for the admin step: DB_CONTAINER (default forum-test-db).
The server must write its mail to the file FORUM_MAIL_FILE (default: forum-mail.txt in the temp
folder) with FORUM_MAIL_MODE=file, because the confirmation links are read from there.
Only standard library. Exit code 1 if a check fails.
"""
import http.cookiejar, os, re, struct, subprocess, sys, tempfile, urllib.error, urllib.parse, urllib.request, uuid, zlib

BASE = os.environ.get("FORUM_URL", "http://127.0.0.1:8090")
DB = os.environ.get("DB_CONTAINER", "forum-test-db")
MAIL = os.environ.get("FORUM_MAIL_FILE") or os.path.join(tempfile.gettempdir(), "forum-mail.txt")
failures = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail and not ok else ""))
    if not ok:
        failures.append(name)


class Client:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar), NoRedirect)

    def get(self, path):
        return self.send(urllib.request.Request(BASE + path))

    def post(self, path, fields, files=None):
        """Thread and reply forms: multipart, because they carry files."""
        body, ctype = encode(fields, files or [])
        return self.send(urllib.request.Request(BASE + path, data=body, headers={"Content-Type": ctype}))

    def post_form(self, path, fields):
        """All other forms: application/x-www-form-urlencoded."""
        body = urllib.parse.urlencode(fields).encode()
        return self.send(urllib.request.Request(BASE + path, data=body,
                                                headers={"Content-Type": "application/x-www-form-urlencoded"}))

    def send(self, request):
        try:
            with self.opener.open(request, timeout=30) as r:
                return r.status, lower(r.headers), r.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            return e.code, lower(e.headers), e.read().decode("utf-8", "replace")


def lower(headers):
    return {key.lower(): value for key, value in headers.items()}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None

    def http_error_302(self, req, fp, code, msg, headers):
        return fp

    http_error_303 = http_error_302


def encode(fields, files):
    boundary = uuid.uuid4().hex
    out = b""
    for name, value in fields.items():
        out += f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n".encode() + value.encode() + b"\r\n"
    for name, filename, content_type, data in files:
        out += (f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                f"Content-Type: {content_type}\r\n\r\n").encode() + data + b"\r\n"
    out += f"--{boundary}--\r\n".encode()
    return out, f"multipart/form-data; boundary={boundary}"


def png(width=2, height=2):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + b"\xff\x00\x00" * width for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def captcha_answer(html):
    m = re.search(r"What is (\d+) \+ (\d+)\?", html)
    return int(m.group(1)) + int(m.group(2)) if m else None


def field(html, name):
    m = re.search(rf'name="{name}" value="([^"]*)"', html)
    return m.group(1) if m else None


def sql(statement):
    return subprocess.run(["docker", "exec", DB, "psql", "-U", "postgres", "-d", "forum", "-tAc", statement],
                          capture_output=True, text=True, check=True).stdout.strip()


def register(client, username, password="correct-horse-9"):
    _, _, html = client.get("/register")
    answer = captcha_answer(html)
    cid = field(html, "captcha_id")
    return client.post_form("/register", {"username": username, "email": f"{username}@example.org",
                                          "password": password, "captcha_id": cid, "captcha_answer": str(answer)})


def latest_mail(address):
    """The newest message the forum wrote to `address` (empty if there is none)."""
    with open(MAIL, encoding="utf-8") as f:
        mails = [m for m in f.read().split("\n----\n") if f"To: {address}\n" in m]
    return mails[-1] if mails else ""


def confirm_latest_link(client, username):
    """Opens the newest confirmation link the forum mailed to `username`'s address."""
    tokens = re.findall(r"/verify\?token=([0-9a-f]{64})", latest_mail(f"{username}@example.org"))
    return client.get(f"/verify?token={tokens[-1]}" if tokens else "/verify")


def log_in(client, username):
    return client.post_form("/login", {"username": username, "password": "correct-horse-9"})


def main():
    suffix = uuid.uuid4().hex[:6]
    alice, bob = Client(), Client()

    status, _, html = alice.get("/register")
    check("register page shows an anti-bot question", status == 200 and captcha_answer(html) is not None)
    s, _, body = alice.post_form("/register", {"username": f"alice_{suffix}", "password": "correct-horse-9",
                                          "captcha_id": field(html, "captcha_id"), "captcha_answer": "999"})
    check("a wrong anti-bot answer is refused", s == 422, str(s))
    s, _, sent = register(alice, f"alice_{suffix}")
    check("registration asks for the e-mail address to be confirmed", s == 200 and "Check your e-mail" in sent, str(s))
    s, h, _ = log_in(alice, f"alice_{suffix}")
    check("an unconfirmed account cannot log in", s == 403 and "forum_session=" not in h.get("set-cookie", ""), str(s))
    s, _, page = confirm_latest_link(alice, f"alice_{suffix}")
    check("the link in the mail confirms the address", s == 200 and "Your address is confirmed" in page, str(s))
    s, h, _ = log_in(alice, f"alice_{suffix}")
    check("a confirmed account logs in", s == 303 and "forum_session=" in h.get("set-cookie", ""), str(s))
    s, _, _ = alice.get("/admin/reports")
    check("the first account ever made is the admin", s == 200, str(s))
    # The other checks use alice as an ordinary member, so the admin role is taken away again here.
    sql(f"UPDATE users SET is_admin = false WHERE username = 'alice_{suffix}'")
    s, _, page = confirm_latest_link(alice, f"alice_{suffix}")
    check("a confirmation link works only once", "has expired or was already used" in page, str(s))

    s, _, sent = register(bob, f"bob_{suffix}")
    if s != 200:
        print("  bob response:", re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", sent))[:300])
    check("a second account asks for confirmation too", s == 200 and "Check your e-mail" in sent, str(s))
    s, _, page = bob.post_form("/verify/resend", {"email": f"bob_{suffix}@example.org"})
    check("a new confirmation link can be requested", s == 200 and "is on its way" in page, str(s))
    s, _, page = confirm_latest_link(bob, f"bob_{suffix}")
    s, h, _ = log_in(bob, f"bob_{suffix}")
    check("the second account logs in after confirming", s == 303 and "forum_session=" in h.get("set-cookie", ""), str(s))
    admin_status, _, _ = bob.get("/admin/reports")
    check("a later account is an ordinary member", admin_status == 403, str(admin_status))

    s, _, form_page = alice.get("/boards/general/threads/new")
    csrf = field(form_page, "csrf") or ""
    title = f"Hallo <script>x</script> {suffix}"
    body_text = "See https://example.org/page. <img src=x onerror=alert(1)>\nSecond line"
    s, h, created_body = alice.post("/boards/general/threads/new",
                                    {"csrf": csrf, "title": title, "body": body_text},
                                    files=[("files", "pic.png", "image/png", png())])
    if s != 303:
        print("  thread response:", re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", created_body))[:300])
    check("a thread with an image is created", s == 303 and "/threads/" in h.get("location", ""), str(s))
    thread_id = int(re.search(r"/threads/(\d+)", h.get("location", "")).group(1)) if s == 303 else 0

    s, _, page = alice.get(f"/threads/{thread_id}")
    check("post text is escaped", "&lt;img src=x onerror=alert(1)&gt;" in page and "<img src=x" not in page)
    check("a plain URL is linked with nofollow", 'href="https://example.org/page"' in page
          and "nofollow ugc noopener noreferrer" in page)
    check("the image is shown through /files/", re.search(r'src="/files/[0-9a-f]{32}"', page) is not None)
    file_id = re.search(r'src="/files/([0-9a-f]{32})"', page).group(1)
    s, h, data = alice.get(f"/files/{file_id}")
    check("the attachment is served as its image type", s == 200 and h.get("content-type") == "image/png")
    check("the attachment has nosniff and a sandbox policy",
          h.get("x-content-type-options") == "nosniff" and "sandbox" in h.get("content-security-policy", ""))

    s, _, body = alice.post(f"/boards/general/threads/new", {"csrf": csrf, "title": "Fake", "body": "x"},
                            files=[("files", "evil.png", "image/png", b"<svg onload=alert(1)>")])
    check("an SVG renamed to .png is refused", s == 422, str(s))

    s, h, _ = alice.post(f"/threads/{thread_id}/reply", {"csrf": csrf, "body": "A reply"},
                         files=[("files", "a.png", "image/png", png()), ("files", "b.png", "image/png", png(3, 3))])
    check("a reply with two images is saved", s == 303, str(s))
    s, _, page = alice.get(f"/threads/{thread_id}")
    check("both reply images appear", len(re.findall(r'class="attachment"', page)) >= 3)
    post_ids = [int(x) for x in re.findall(r'id="post-(\d+)"', page)]
    own_post = post_ids[-1]
    s, _, _ = alice.post_form(f"/posts/{own_post}/delete", {"csrf": csrf, "thread": str(thread_id)})
    s, _, page = alice.get(f"/threads/{thread_id}")
    check("a deleted post shows as removed", "This post was removed." in page)

    s, _, bobpage = bob.get(f"/threads/{thread_id}")
    bob_csrf = field(bobpage, "csrf") or ""
    s, _, _ = bob.post_form(f"/posts/{post_ids[0]}/report", {"csrf": bob_csrf, "thread": str(thread_id), "reason": "spam"})
    check("a post can be reported", s == 303, str(s))

    admin_name = f"bob_{suffix}"
    sql(f"UPDATE users SET is_admin = true WHERE username = '{admin_name}'")
    _, _, reports = bob.get("/admin/reports")
    check("the admin sees the report queue", "reported by" in reports)
    admin_csrf = field(reports, "csrf") or ""
    s, _, _ = bob.post_form(f"/admin/posts/{post_ids[0]}/remove", {"csrf": admin_csrf})
    _, _, page = bob.get(f"/threads/{thread_id}")
    check("the admin can remove a post", "This post was removed." in page and s == 303)

    s, _, _ = alice.get("/admin/reports")
    check("a normal user cannot open the report queue", s == 403, str(s))

    _, _, search = alice.get("/search?q=" + urllib.parse.quote(suffix))
    check("search finds the thread by its title", suffix in search)
    _, _, profile = alice.get(f"/users/alice_{suffix}")
    check("the profile page lists the member", f"alice_{suffix}" in profile)

    community(alice, bob, suffix)
    reactions_and_seo(alice, bob, suffix)
    metadata_and_deletion(alice, suffix)

    print("\n%d check(s) failed" % len(failures) if failures else "\nall checks passed")
    return 1 if failures else 0


def reactions_and_seo(alice, bob, suffix):
    """Reaction kinds that an admin adds, reactions that members give and take back, tabs in an
    embed, and what search engines read: the canonical address, robots.txt, the sitemap, noindex."""
    alice_name = f"alice_{suffix}"
    s, _, _ = alice.post_form("/login", {"username": alice_name, "password": "fresh-horse-42"})
    check("the member signs in again with the new password", s == 303, str(s))

    _, _, admin_page = bob.get("/admin/reactions")
    admin_csrf = field(admin_page, "csrf") or ""
    s, _, page = alice.post_form("/admin/reactions", {"csrf": admin_csrf, "symbol": "!", "name": "Nope"})
    check("a member cannot add reactions", s == 403, str(s))
    s, _, _ = bob.post_form("/admin/reactions", {"csrf": admin_csrf, "symbol": "👍", "name": f"Thumbs {suffix}"})
    check("an admin adds a reaction kind", s == 303, str(s))
    _, _, admin_page = bob.get("/admin/reactions")
    check("the kind is listed for the admin", f"Thumbs {suffix}" in admin_page)

    _, _, form_page = bob.get("/boards/general/threads/new")
    body = '[embed title="Install"][tabs][tab=Linux]run it[/tab][tab=Windows]click it[/tab][/tabs][/embed]'
    s, h, _ = bob.post("/boards/general/threads/new", {"csrf": field(form_page, "csrf") or "",
                                                     "title": f"Install guide {suffix}", "body": body})
    check("a thread with tabs in an embed is created", s == 303, str(s))
    thread_id = re.search(r"/threads/(\d+)", h.get("location", "")).group(1) if s == 303 else "0"

    _, _, page = alice.get(f"/threads/{thread_id}")
    check("tabs in an embed render as radio tabs with panels",
          'class="tabs"' in page and page.count('type="radio"') == 2 and 'class="p2"' in page)
    slug = f"install-guide-{suffix}"
    check("the page names its canonical address with the title words",
          f'rel="canonical" href="{BASE}/threads/{thread_id}/{slug}"' in page)
    check("the page has a description", 'name="description"' in page)
    s, _, page_titled = alice.get(f"/threads/{thread_id}/a-different-title")
    check("the same thread opens under any title words", s == 200 and "Install guide" in page_titled, str(s))

    post_id = re.search(r'/posts/(\d+)/react', page).group(1)
    kind_id = re.search(r'name="kind" value="(\d+)"', page).group(1)
    alice_csrf = field(page, "csrf") or ""
    s, _, _ = alice.post_form(f"/posts/{post_id}/react", {"csrf": alice_csrf, "kind": kind_id})
    check("a member gives a reaction", s == 303, str(s))
    _, _, page = alice.get(f"/threads/{thread_id}")
    check("the reaction is counted and marked as the member's own", 'class="reaction mine"' in page and ">1<" in page)
    s, _, _ = alice.post_form(f"/posts/{post_id}/react", {"csrf": alice_csrf, "kind": kind_id})
    _, _, page = alice.get(f"/threads/{thread_id}")
    check("giving the reaction again takes it back", 'class="reaction mine"' not in page)

    anon = Client()
    s, _, robots = anon.get("/robots.txt")
    check("robots.txt keeps the private pages out and names the sitemap",
          s == 200 and "Disallow: /admin" in robots and f"Sitemap: {BASE}/sitemap.xml" in robots)
    s, _, sitemap = anon.get("/sitemap.xml")
    check("the sitemap lists the thread under its address", s == 200 and f"/threads/{thread_id}/{slug}" in sitemap)
    s, h, _ = alice.get("/settings")
    check("private pages send a noindex header", "noindex" in h.get("x-robots-tag", ""), h.get("x-robots-tag", ""))
    _, h, _ = anon.get(f"/threads/{thread_id}")
    check("public pages are not marked noindex", "x-robots-tag" not in h)


def png_with_text(comment):
    """A PNG with a text chunk that names a place, as a camera or an editor may add."""
    base = png()
    body = b"Comment\0" + comment.encode()
    chunk = struct.pack(">I", len(body)) + b"tEXt" + body + b"\0\0\0\0"
    end = base.rfind(b"IEND") - 4
    return base[:end] + chunk + base[end:]


def metadata_and_deletion(alice, suffix):
    """Uploaded pictures lose their text metadata; a member can delete the account, and its posts stay
    under [deleted]."""
    _, _, form_page = alice.get("/boards/general/threads/new")
    s, h, _ = alice.post("/boards/general/threads/new", {"csrf": field(form_page, "csrf") or "",
                                                       "title": f"Metadata {suffix}", "body": "A picture with a place"},
                         files=[("files", "place.png", "image/png", png_with_text("taken at secret place"))])
    check("a picture with a text chunk is accepted", s == 303, str(s))
    thread_id = re.search(r"/threads/(\d+)", h.get("location", "")).group(1) if s == 303 else "0"
    _, _, page = alice.get(f"/threads/{thread_id}")
    file_id = re.search(r'src="/files/([0-9a-f]{32})"', page)
    if file_id:
        _, _, data = alice.get(f"/files/{file_id.group(1)}")
        check("the stored picture has no text metadata", "secret place" not in data and "tEXt" not in data)
    else:
        check("the stored picture has no text metadata", False, "no picture on the page")

    dave, dave_name = Client(), f"dave_{suffix}"
    register(dave, dave_name)
    confirm_latest_link(dave, dave_name)
    dave.post_form("/login", {"username": dave_name, "password": "correct-horse-9"})
    _, _, form_page = dave.get("/boards/general/threads/new")
    dave.post("/boards/general/threads/new", {"csrf": field(form_page, "csrf") or "",
                                             "title": f"Deleted member {suffix}", "body": "Still here after the account is gone"})
    _, _, form = dave.get("/settings/delete")
    s, _, _ = dave.post_form("/settings/delete", {"csrf": field(form, "csrf") or "", "password": "wrong-password-1"})
    check("the deletion asks for the password again", s == 422, str(s))
    s, h, _ = dave.post_form("/settings/delete", {"csrf": field(form, "csrf") or "", "password": "correct-horse-9"})
    check("the account is deleted with its password", s == 303, str(s))
    s, _, _ = log_in(dave, dave_name)
    check("a deleted account cannot log in", s == 401, str(s))
    _, _, search = alice.get("/search?q=" + urllib.parse.quote(f"Deleted member {suffix}"))
    check("the thread stays, under [deleted]", f"Deleted member {suffix}" in search)
    s, _, _ = alice.get(f"/users/{dave_name}")
    check("the profile is gone", s == 404, str(s))


def community(alice, bob, suffix):
    """Quotes, embeds and polls; mentions and replies; bans and titles; pictures; private messages;
    and a new password. Bob is the admin by now."""
    alice_name, bob_name = f"alice_{suffix}", f"bob_{suffix}"

    _, _, form_page = alice.get("/boards/general/threads/new")
    body_text = ('[quote=alice_x]quoted text[/quote]\n'
                 '[embed title="Card" url="https://example.org" color="blue"]card text[/embed]\n'
                 '[spoiler]hidden[/spoiler] [b]bold[/b] <i>not markup</i>')
    s, h, _ = alice.post("/boards/general/threads/new", {"csrf": field(form_page, "csrf") or "",
                                                       "title": f"Poll {suffix}", "body": body_text,
                                                       "poll_question": "Best?", "poll_options": "Rust\nGo"})
    check("a thread with a poll is created", s == 303, str(s))
    thread_id = int(re.search(r"/threads/(\d+)", h.get("location", "")).group(1)) if s == 303 else 0

    _, _, page = alice.get(f"/threads/{thread_id}")
    check("quotes, embeds and spoilers are rendered",
          "<blockquote" in page and 'class="embed blue"' in page and 'class="spoiler"' in page)
    check("the markup is escaped where it is not a tag", "&lt;i&gt;not markup&lt;/i&gt;" in page)
    check("the poll shows its options", 'class="poll"' in page and ">Rust<" in page and ">Go<" in page)
    poll_id = re.search(r"/polls/(\d+)/vote", page).group(1)
    option_id = re.search(r'name="option" value="(\d+)"', page).group(1)

    _, _, bobpage = bob.get(f"/threads/{thread_id}")
    bob_csrf = field(bobpage, "csrf") or ""
    s, _, _ = bob.post_form(f"/polls/{poll_id}/vote", {"csrf": bob_csrf, "option": option_id})
    check("a member votes in a poll", s == 303, str(s))
    _, _, page = bob.get(f"/threads/{thread_id}")
    check("the result counts the vote", "1 vote" in page)

    s, _, _ = bob.post(f"/threads/{thread_id}/reply", {"csrf": bob_csrf, "body": f"Hello @alice_{suffix}, see the poll"})
    check("a reply with a mention is saved", s == 303, str(s))
    _, _, notes = alice.get("/notifications")
    check("the mentioned member is told", "mentioned you in" in notes)
    s, _, _ = bob.post_form(f"/threads/{thread_id}/follow", {"csrf": bob_csrf, "action": "follow"})
    s, _, _ = bob.post(f"/threads/{thread_id}/reply", {"csrf": bob_csrf, "body": "A plain reply"})
    _, _, notes = alice.get("/notifications")
    check("a follower of the thread hears about the reply", "replied in" in notes)
    _, _, page = alice.get("/")
    check("unread notifications are counted in the header", 'class="count"' in page)

    _, _, admin_page = bob.get(f"/users/{alice_name}")
    admin_csrf = field(admin_page, "csrf") or ""
    s, _, _ = bob.post_form(f"/admin/users/{alice_name}/title", {"csrf": admin_csrf, "title": "Helper"})
    _, _, profile = bob.get(f"/users/{alice_name}")
    check("an admin sets a title that the profile shows", "Helper" in profile, str(s))
    s, _, _ = bob.post_form(f"/admin/users/{alice_name}/ban", {"csrf": admin_csrf, "duration": "1", "reason": "test"})
    check("an admin bans a member", s == 303, str(s))
    s, h, _ = alice.get("/notifications")
    check("a banned member is signed out", s == 303 and "/login" in h.get("location", ""), str(s))
    s, _, body = log_in(alice, alice_name)
    check("a banned member cannot log in", s == 403 and "is banned" in body, str(s))
    bob.post_form(f"/admin/users/{alice_name}/unban", {"csrf": admin_csrf})
    s, _, _ = log_in(alice, alice_name)
    check("lifting the ban lets the member log in again", s == 303, str(s))

    _, _, settings_page = alice.get("/settings")
    s, _, _ = alice.post("/settings/avatar", {"csrf": field(settings_page, "csrf") or ""},
                         files=[("avatar", "me.png", "image/png", png())])
    check("a member sets a picture", s == 303, str(s))
    _, _, profile = alice.get(f"/users/{alice_name}")
    picture = re.search(r"/avatars/([0-9a-f]{32})", profile)
    check("the picture is on the profile", picture is not None)
    if picture:
        s, h, _ = alice.get(f"/avatars/{picture.group(1)}")
        check("the picture is served as an image", s == 200 and h.get("content-type") == "image/png", str(s))

    sql("INSERT INTO settings (key, value) VALUES ('private_messages', 'off') ON CONFLICT (key) DO UPDATE SET value = 'off'")
    s, _, page = alice.get("/messages")
    check("private messages are off by default", s == 403 and "switched off" in page, str(s))
    sql("UPDATE settings SET value = 'on' WHERE key = 'private_messages'")
    _, _, compose = alice.get(f"/messages/new/{bob_name}")
    s, h, _ = alice.post_form(f"/messages/new/{bob_name}", {"csrf": field(compose, "csrf") or "",
                                                           "subject": f"Hi {suffix}", "body": "A private note"})
    check("a private message is sent when the setting is on", s == 303 and "/messages/" in h.get("location", ""), str(s))
    conversation = re.search(r"/messages/(\d+)", h.get("location", "")).group(1) if s == 303 else "0"
    _, _, inbox = bob.get("/messages")
    check("the recipient sees it in the inbox", f"Hi {suffix}" in inbox)
    s, _, _ = bob.post_form(f"/messages/{conversation}/reply", {"csrf": bob_csrf, "body": "Got it"})
    _, _, page = alice.get(f"/messages/{conversation}")
    check("a reply is part of the conversation", "Got it" in page)

    anon = Client()
    s, _, page = anon.post_form("/password/forgot", {"email": f"{alice_name}@example.org"})
    check("the reset form gives the same answer for any address", s == 200 and "is on its way" in page, str(s))
    token = re.search(r"/password/reset\?token=([0-9a-f]{64})", latest_mail(f"{alice_name}@example.org"))
    if token:
        s, _, page = anon.post_form("/password/reset", {"token": token.group(1), "password": "fresh-horse-42"})
        check("a new password is set with the link", s == 200 and "Password changed" in page, str(s))
        s, _, page = anon.post_form("/password/reset", {"token": token.group(1), "password": "again-horse-43"})
        check("a reset link works once", "cannot be used" in page, str(s))
        s, _, _ = anon.post_form("/login", {"username": alice_name, "password": "fresh-horse-42"})
        check("the new password logs in", s == 303, str(s))
        s, _, _ = anon.post_form("/login", {"username": alice_name, "password": "correct-horse-9"})
        check("the old password no longer works", s == 401, str(s))
    else:
        check("the reset mail has a link", False)


if __name__ == "__main__":
    sys.exit(main())
