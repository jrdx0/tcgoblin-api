# syntax=docker/dockerfile:1

FROM rust:1-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bins \
    && mkdir /out \
    && cp target/release/api target/release/worker target/release/scheduler /out/

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates chromium fonts-liberation tzdata \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --create-home --uid 10001 app
COPY --from=build /out/ /usr/local/bin/
USER app
WORKDIR /home/app
ENV CHROME_EXECUTABLE=/usr/bin/chromium \
    CHROME_NO_SANDBOX=true
EXPOSE 3000
CMD ["api"]
