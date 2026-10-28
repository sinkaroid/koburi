# AGENTS.md

## koburi

A single binary that exposes container resource stats (equivalent to
`docker stats --no-stream --format json` / `podman stats --no-stream --format json`)
over a small HTTP API. Target environment: memory-constrained VPS (4GB, many
containers running). The tool itself must not become part of the resource
problem it's meant to monitor.

Primary metric of success: **low idle RSS**, not disk binary size, not
feature count, not benchmark throughput.

## Non-negotiable architecture rules

These override "idiomatic Rust" or "what's easier to write" every time.
If a change makes the code nicer but adds even one of the violations below,
reject it.

1. **No async runtime.**
   This serves one or two endpoints, hit occasionally (polling interval,
   not high QPS). `tokio`/`async-std` reserve worker threads and an
   executor up front — that's MBs of RSS paid at idle for concurrency
   this workload doesn't need. Use `std::net::TcpListener` with blocking
   I/O. One thread per connection (or even fully sequential accept loop)
   is fine at this request rate.

2. **No full-buffer reads of stats payloads.**
   Never `read_to_string` / collect the entire Docker/Podman socket
   response into memory before processing. Stream-parse: read off the
   `UnixStream` in fixed-size chunks and parse/forward incrementally.
   This matters most for the _streaming_ stats endpoint if ever touched
   — but even for `stream=false` snapshots, don't allocate more than
   one bounded buffer per request. No `Vec<u8>` that grows unbounded
   waiting for EOF.

3. **Socket connections are on-demand, not persistent.**
   Open the Unix socket connection (`/var/run/docker.sock` or
   `/run/podman/podman.sock`) per request, use it, close it. Do not
   hold a long-lived connection or connection pool "for efficiency" —
   at this request rate the connect() syscall cost is irrelevant, and
   a persistent connection is a resource held (and a thing that can go
   stale) for no benefit.

4. **musl static linking.**
   Build target is `x86_64-unknown-linux-musl` (or the appropriate musl
   target for the deploy arch). Static linking avoids glibc's baseline
   heap arena overhead and dynamic linker resolution cost. The release
   binary must run with zero dynamic library dependencies (`ldd` should
   report "not a dynamic executable").

5. **No dependencies.**
   `Cargo.toml` has no `[dependencies]`. Standard library only:
   `std::net`, `std::os::unix::net::UnixStream`, `std::io`, `std::thread`.
   HTTP parsing/serialization and JSON encoding are hand-rolled. This is
   deliberate — every crate pulled in is unaudited weight and an
   unpredictable allocator/thread behavior at idle. If a feature seems
   to require a dependency, the feature is cut or written by hand, not
   the rule.

## What this tool does

- Detects which container runtime socket is present/reachable:
  `/var/run/docker.sock` first, then `/run/podman/podman.sock`
  (rootful), then `/run/user/$UID/podman/podman.sock` (rootless).
  Configurable override via env var, not auto-guessed silently in prod.
- On `GET /stats`: queries `containers/json` (list running containers),
  then for each, queries `containers/{id}/stats?stream=false` (Docker)
  or the libpod-compatible equivalent (Podman), computes CPU % from
  `cpu_stats`/`precpu_stats` deltas the same way `docker stats` does,
  and returns a JSON array — one object per container — mirroring the
  shape of `docker stats --format json` (`Name`, `CPUPerc`, `MemUsage`,
  `MemPerc`, `NetIO`, `BlockIO`, `PIDs`, plus `ID` and `Runtime` to
  disambiguate Docker vs Podman when both are queried).
- On `GET /stats/{id}`: same, scoped to a single container.
- No write/mutating endpoints. This tool only reads.

## Build

```
cargo build --release --target x86_64-unknown-linux-musl
strip target/x86_64-unknown-linux-musl/release/<bin>
```

Release profile in `Cargo.toml`:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

## Verifying compliance before merging any change

- `ldd target/.../release/<bin>` → must say "not a dynamic executable".
- `cat Cargo.toml` → `[dependencies]` section must be empty or absent.
- Idle RSS check: start the binary, hit it with zero requests for 30s,
  check `ps -o rss -p $(pgrep <bin>)`. Should be single-digit MB.
- `grep -r "tokio\|async-std\|async fn" src/` → must return nothing.
- No connection object outlives a single request's handler function.

## Style notes

- Prefer explicit, boring blocking I/O code over clever abstractions.
  This codebase optimizes for predictable memory behavior, not for
  being pleasant to extend — don't add trait objects, generic handler
  registries, or middleware layers "for future endpoints." Add
  endpoints by adding a match arm.
- Errors: return a plain JSON `{"error": "..."}" with an appropriate
  HTTP status, don't panic on malformed upstream responses (a
  container being removed mid-request is expected, not exceptional).
