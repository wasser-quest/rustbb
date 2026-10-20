# syntax=docker/dockerfile:1
# Pure Rust: no system libraries are needed to build.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
COPY static ./static
RUN cargo build --release --locked

# The runtime image adds msmtp, which hands the forum's mail to the SMTP relay. The relay's
# settings and password are in /etc/msmtprc, mounted at run time: they are not part of the image.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends msmtp msmtp-mta ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin rustbb
COPY --from=build /src/target/release/rustbb /usr/local/bin/rustbb
ENV FORUM_BIND=0.0.0.0:8080 \
    FORUM_MAIL_MODE=sendmail \
    FORUM_SENDMAIL=/usr/sbin/sendmail
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/rustbb"]
