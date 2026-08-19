# The headless half of BAKA in a container: watch, serve and files. The interface needs
# a terminal, so `docker run -it baka` drops you into it but a detached container should
# be given one of the three server subcommands instead.
#
#   docker build -t baka .
#   docker run --rm -v "$PWD/config:/home/baka/.config/baka" \
#                   -v "$PWD/downloads:/home/baka/downloads" \
#                   -p 4241:4241 -p 4242:4242 baka serve
#
# The bind address defaults to loopback, so the mounted config.toml has to set it to
# 0.0.0.0 before anything outside the container can reach the intake.
#
# Both bases are pinned to bookworm so the binary and the glibc it was linked against
# stay the same version.
FROM rust:bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
# Every source BAKA queries is HTTPS, so a root store is not optional.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 baka
COPY --from=build /src/target/release/baka /usr/local/bin/baka
USER baka
WORKDIR /home/baka
# 4240 peers, 4241 magnet intake, 4242 finished files. Changing any of them is a
# Settings page edit, not a flag, so a changed port needs the mapping changed too.
EXPOSE 4240 4241 4242
ENTRYPOINT ["baka"]
CMD ["serve"]
