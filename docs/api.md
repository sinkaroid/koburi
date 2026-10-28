## Deploying with Docker / Podman

You can run `koburi` as a container. Since it needs to talk to the container runtime socket, you must mount the socket inside the container.

### Manual

```sh
cargo build --release --target x86_64-unknown-linux-musl
strip target/x86_64-unknown-linux-musl/release/koburi
KOBURI_PORT=9911 ./koburi
```

### Docker

```sh
docker run -d \
  --name koburi \
  --restart unless-stopped \
  -p 9911:9911 \
  -u root \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v /run/podman/podman.sock:/run/podman/podman.sock:ro \
  -v /root/.pm2/rpc.sock:/root/.pm2/rpc.sock:ro \
  -e SOCKET_DOCKER=/var/run/docker.sock \
  -e SOCKET_PODMAN=/run/podman/podman.sock \
  -e SOCKET_PM2=/root/.pm2/rpc.sock \
  ghcr.io/sinkaroid/koburi:latest
```

### Podman

Simplified with `--net=host` for firewall/UFW stuff / or if you don't want to, set manual

```sh
podman run -d \
  --name koburi \
  --restart unless-stopped \
  --net=host \
  -u root \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v /run/podman/podman.sock:/run/podman/podman.sock:ro \
  -v /root/.pm2/rpc.sock:/root/.pm2/rpc.sock:ro \
  -e SOCKET_DOCKER=/var/run/docker.sock \
  -e SOCKET_PODMAN=/run/podman/podman.sock \
  -e SOCKET_PM2=/root/.pm2/rpc.sock \
  ghcr.io/sinkaroid/koburi:latest
```

## Endpoints

All responses are `application/json`. Errors are `{"error": "..."}` with
an appropriate HTTP status.

### `GET /`

Index. Lists the available endpoints.

```sh
curl -s http://127.0.0.1:9911/
```

```json
{ "endpoints": ["/health", "/stats", "/stats/{id}"] }
```

### `GET /health`

Liveness probe. Does not touch the container runtime. Returns 200 if the
HTTP listener is up.

```sh
curl -s http://127.0.0.1:9911/health
```

```json
{ "ok": true }
```

### `GET /stats`

Returns one JSON object per running container/process, in the same shape as
`docker stats --format json`, plus `ID` and `Runtime` to disambiguate
when docker, podman, and/or pm2 are queried.

```sh
curl -s http://127.0.0.1:9911/stats
```

```json
[
  {
    "ID": "a1b2c3d4e5f6...",
    "Runtime": "docker",
    "Name": "myapp",
    "Uptime": "Up 2 hours",
    "CPUPerc": "12.34%",
    "MemUsage": "142.6MiB / 3.8GiB",
    "MemPerc": "3.66%",
    "NetIO": "1.2MB / 768.4kB",
    "BlockIO": "12.3MB / 4.5MB",
    "PIDs": 23
  }
]
```

Containers that disappear mid-request are skipped (no 500 on the batch).

### `GET /stats/{id}`

Same as `/stats` but scoped to a single container ID (the long hex ID
returned by docker, or the podman container ID).

```sh
curl -s http://127.0.0.1:9911/stats/a1b2c3d4e5f6
```

Returns the same single-object shape as one element of the `/stats`
array, or `404 {"error": "container not found"}` if the ID is not in
the running container list.

## Status codes

- `200` — success.
- `400` — malformed ID in `/stats/{id}` (empty, contains `/`, too long).
- `404` — `/stats/{id}` with an ID that is not currently running, or
  any unknown path.
- `502` — the upstream container runtime returned an error or
  unparseable response.
- `503` — no container runtime socket could be resolved.

## Compliance

This binary is built per the rules in [AGENTS.md](AGENTS.md). No
runtime dependencies, no async runtime, no long-lived upstream
connections, static musl build.
