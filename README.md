# decompressor

`decompressor` is a small reverse proxy that sits in front of upstream services or intermediate proxies that do not correctly support compressed HTTP request bodies.

Its main job is to normalize incoming request bodies before they reach the next hop:

- if the incoming request body uses `Content-Encoding: gzip`, `decompressor` will stream-decompress it before forwarding upstream
- if the incoming request body is not compressed, `decompressor` forwards it as-is
- upstream responses are streamed back to the client unchanged

## What This Replaces

If your original `docker-compose.yml` looked like this:

```yaml
services:
  upstream-service:
    image: your/upstream-image:latest
    container_name: upstream-service
    restart: unless-stopped
    ports:
      - "127.0.0.1:5505:8080"
```

then the host port `5505` was pointing directly to the upstream service.

With this project, `decompressor` takes over that public port, and the upstream service stays internal.

## Recommended Compose Setup

Use this project directory as the compose root and run:

```yaml
services:
  upstream-service:
    image: your/upstream-image:latest
    container_name: upstream-service
    restart: unless-stopped
    expose:
      - "8080"

  decompressor:
    image: ghcr.io/550w-host/decompressor:latest
    container_name: decompressor
    restart: unless-stopped
    depends_on:
      - upstream-service
    env_file:
      - ./.env
    ports:
      - "127.0.0.1:5505:5505"
```

That same setup is also included in [docker-compose.yml](/home/vivy/compressor/docker-compose.yml).

## Getting The Repo

```bash
git clone https://github.com/550W-HOST/decompressor.git
cd decompressor
```

## How It Works

Client traffic now flows like this:

```text
client -> localhost:5505 -> decompressor -> upstream-service:8080
```

Request handling rules:

- `Content-Encoding: gzip`: streamed through a gzip decoder, then forwarded upstream without the `Content-Encoding` header
- no request compression: forwarded directly
- responses: streamed straight back to the client

## Environment Variables

`decompressor` reads its runtime settings from environment variables. In local development and Docker Compose, the easiest way is to keep them in `.env`.

Example:

```env
TEST_API_BASE=https://your-api.example.com
TEST_API_KEY=your_api_key_here
TEST_MODEL=gpt-5.2
DECOMPRESSOR_LISTEN_ADDR=0.0.0.0:5505
DECOMPRESSOR_UPSTREAM_URL=http://upstream-service:8080
```

The sample values are also present in [.env.example](/home/vivy/compressor/.env.example).

## Build And Run

Start everything:

```bash
docker compose up -d
```

Once it is running, point your client at:

```text
http://127.0.0.1:5505
```

## Local Development

Run the Rust service directly:

```bash
cargo run
```

By default it listens on `0.0.0.0:5505` and forwards to `http://127.0.0.1:8080`.

Before starting it locally, create `.env` from `.env.example`.

Run tests:

```bash
cargo test
```

The gzip compatibility investigation tests live in [tests/cpa_gzip_request_body.rs](/home/vivy/compressor/tests/cpa_gzip_request_body.rs). The local proxy end-to-end tests live in [tests/proxy_forwarding.rs](/home/vivy/compressor/tests/proxy_forwarding.rs).
