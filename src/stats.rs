// Parse the upstream payloads and compute the same numbers `docker stats`
// emits. CPU% = (cpu_delta / system_delta) * num_cpus * 100, where
// cpu_delta and system_delta come from cpu_stats vs precpu_stats. If the
// deltas are zero or the system delta is zero, the returned percentage is
// 0.0 (matches docker behavior, not NaN).
#![allow(dead_code)]

use std::io;

use crate::json;

#[derive(Debug)]
pub struct Snapshot {
    pub cpu_perc: f64,
    pub mem_usage: u64,
    pub mem_limit: u64,
    pub mem_perc: f64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub block_read: u64,
    pub block_write: u64,
    pub pids: u64,
}

pub fn parse_container_list(body: &[u8]) -> io::Result<Vec<(String, String, String)>> {
    let root = match json::classify(body, 0) {
        Some(s) if s.kind == json::Kind::Array => s,
        Some(s) if s.kind == json::Kind::Object => {
            if let Some(msg_span) = json::object_get(body, s, "message")
                .or_else(|| json::object_get(body, s, "error"))
            {
                if let Ok(msg) = json::unquote(&body[msg_span.start..msg_span.end]) {
                    return Err(io::Error::new(io::ErrorKind::Other, msg));
                }
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "list: not an array",
            ))
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "list: not an array",
            ))
        }
    };
    let mut out = Vec::new();
    json::array_iter(body, root, |item| {
        if item.kind != json::Kind::Object {
            return true;
        }
        let id = json::object_get(body, item, "Id")
            .and_then(|s| json::as_str(body, s))
            .map(str::to_owned)
            .unwrap_or_default();
        let name = json::object_get(body, item, "Names")
            .or_else(|| json::object_get(body, item, "Name"))
            .and_then(|s| {
                if s.kind == json::Kind::Array {
                    let mut found = String::new();
                    json::array_iter(body, s, |n| {
                        if let Some(t) = json::as_str(body, n) {
                            found = t.trim_start_matches('/').to_owned();
                            return false;
                        }
                        true
                    });
                    Some(found)
                } else {
                    json::as_str(body, s).map(|t| t.trim_start_matches('/').to_owned())
                }
            })
            .unwrap_or_else(|| id[..12.min(id.len())].to_owned());
        let status = json::object_get(body, item, "Status")
            .and_then(|s| json::as_str(body, s))
            .map(str::to_owned)
            .unwrap_or_default();
        if !id.is_empty() {
            out.push((id, name, status));
        }
        true
    });
    Ok(out)
}

pub fn compute(body: &[u8]) -> io::Result<Snapshot> {
    let root = match json::classify(body, 0) {
        Some(s) if s.kind == json::Kind::Object => s,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "stats: not object")),
    };

    if let Some(msg_span) = json::object_get(body, root, "message")
        .or_else(|| json::object_get(body, root, "error"))
    {
        if let Ok(msg) = json::unquote(&body[msg_span.start..msg_span.end]) {
            return Err(io::Error::new(io::ErrorKind::Other, msg));
        }
    }

    // CPU%
    let cpu_stats = json::object_get(body, root, "cpu_stats");
    let precpu_stats = json::object_get(body, root, "precpu_stats");
    let (cpu_perc, _num_cpus) = compute_cpu(body, cpu_stats, precpu_stats);

    // Memory
    let memory_stats = json::object_get(body, root, "memory_stats");
    let (mem_usage, mem_limit) = compute_memory(body, memory_stats);
    let mem_perc = if mem_limit > 0 {
        (mem_usage as f64) * 100.0 / (mem_limit as f64)
    } else {
        0.0
    };

    // PIDs
    let pids = json::object_get(body, root, "pids_stats")
        .and_then(|p| json::object_get(body, p, "current"))
        .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
        .unwrap_or(0);

    // Networks & BlockIO
    let (net_rx, net_tx) = sum_networks(body, root);
    let (block_read, block_write) = sum_blkio(body, root);

    Ok(Snapshot {
        cpu_perc,
        mem_usage,
        mem_limit,
        mem_perc,
        net_rx,
        net_tx,
        block_read,
        block_write,
        pids,
    })
}

fn compute_cpu(
    body: &[u8],
    cpu_stats: Option<json::Span>,
    precpu_stats: Option<json::Span>,
) -> (f64, u64) {
    let Some(cpu) = cpu_stats else { return (0.0, 0) };
    let Some(precpu) = precpu_stats else { return (0.0, 0) };

    let cpu_total = json::object_get(body, cpu, "cpu_usage")
        .and_then(|u| json::object_get(body, u, "total_usage"))
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0);
    let pre_cpu_total = json::object_get(body, precpu, "cpu_usage")
        .and_then(|u| json::object_get(body, u, "total_usage"))
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0);
    let sys_total = json::object_get(body, cpu, "system_cpu_usage")
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0);
    let pre_sys_total = json::object_get(body, precpu, "system_cpu_usage")
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0);
    let num_cpus = json::object_get(body, cpu, "online_cpus")
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .map(|v| v as u64)
        .unwrap_or(1);

    let cpu_delta = cpu_total - pre_cpu_total;
    let sys_delta = sys_total - pre_sys_total;
    let perc = if cpu_delta > 0.0 && sys_delta > 0.0 {
        (cpu_delta / sys_delta) * (num_cpus as f64) * 100.0
    } else {
        0.0
    };
    (perc, num_cpus)
}

pub fn get_system_mem() -> u64 {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                let mut parts = line.split_whitespace();
                let _ = parts.next();
                if let Some(kb_str) = parts.next() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb * 1024;
                    }
                }
            }
        }
    }
    0
}

fn compute_memory(body: &[u8], memory: Option<json::Span>) -> (u64, u64) {
    let Some(mem) = memory else { return (0, 0) };
    let raw_usage = json::object_get(body, mem, "usage")
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0) as u64;
    // `limit` may be missing on some kernels; fallback to 0.
    let mut limit = json::object_get(body, mem, "limit")
        .and_then(|s| json::parse_f64(&body[s.start..s.end]))
        .unwrap_or(0.0) as u64;

    let host_mem = get_system_mem();
    if host_mem > 0 && (limit == 0 || limit > host_mem) {
        limit = host_mem;
    }

    // Extract cache or inactive_file from stats to compute actual memory usage (matching docker stats)
    let stats = json::object_get(body, mem, "stats");
    let cache = stats
        .and_then(|s| {
            json::object_get(body, s, "cache")
                .or_else(|| json::object_get(body, s, "inactive_file"))
        })
        .and_then(|span| json::parse_f64(&body[span.start..span.end]))
        .unwrap_or(0.0) as u64;

    let usage = if raw_usage > cache { raw_usage - cache } else { raw_usage };
    (usage, limit)
}

fn sum_networks(body: &[u8], root: json::Span) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    let Some(networks) = json::object_get(body, root, "networks") else {
        return (0, 0);
    };
    if networks.kind != json::Kind::Object {
        return (0, 0);
    }
    // iterate over entries of the networks object
    let body_inner = &body[networks.start + 1..networks.end - 1];
    let mut i = 0;
    while let Some(start) = json_skip_ws(body_inner, i) {
        let Some(k) = json::classify(body_inner, start) else { break };
        if k.kind != json::Kind::String {
            break;
        }
        i = k.end;
        let Some(colon) = json_skip_ws(body_inner, i) else { break };
        if body_inner.get(colon) != Some(&b':') {
            break;
        }
        let Some(vstart) = json_skip_ws(body_inner, colon + 1) else { break };
        let Some(v) = json::classify(body_inner, vstart) else { break };
        if v.kind == json::Kind::Object {
            let abs = json::Span {
                start: networks.start + 1 + v.start,
                end: networks.start + 1 + v.end,
                kind: json::Kind::Object,
            };
            if let Some(r) = json::object_get(body, abs, "rx_bytes")
                .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
            {
                rx = rx.saturating_add(r);
            }
            if let Some(t) = json::object_get(body, abs, "tx_bytes")
                .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
            {
                tx = tx.saturating_add(t);
            }
        }
        i = v.end;
        i = json_skip_ws(body_inner, i).unwrap_or(i);
        if body_inner.get(i) == Some(&b',') {
            i += 1;
        }
    }
    (rx, tx)
}

fn sum_blkio(body: &[u8], root: json::Span) -> (u64, u64) {
    let mut read = 0u64;
    let mut write = 0u64;
    let Some(blkio) = json::object_get(body, root, "blkio_stats") else {
        return (0, 0);
    };
    let Some(svc) = json::object_get(body, blkio, "io_service_bytes_recursive")
        .or_else(|| json::object_get(body, blkio, "io_service_bytes"))
    else {
        return (0, 0);
    };
    if svc.kind != json::Kind::Array {
        return (0, 0);
    }
    let abs = svc;
    json::array_iter(body, abs, |item| {
        if item.kind != json::Kind::Object {
            return true;
        }
        let op = json::object_get(body, item, "op")
            .and_then(|s| json::as_str(body, s))
            .unwrap_or("");
        let val = json::object_get(body, item, "value")
            .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
            .unwrap_or(0);
        match op {
            "Read" | "read" => read = read.saturating_add(val),
            "Write" | "write" => write = write.saturating_add(val),
            _ => {}
        }
        true
    });
    (read, write)
}

fn json_skip_ws(src: &[u8], mut i: usize) -> Option<usize> {
    while i < src.len() {
        match src[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => return Some(i),
        }
    }
    None
}

pub fn append_json(
    out: &mut String,
    id: &str,
    name: &str,
    uptime: &str,
    runtime_label: &str,
    s: &Snapshot,
) {
    use std::fmt::Write;
    let _ = write!(
        out,
        r#"{{"ID":{},"Runtime":"{}","Name":{},"Uptime":{},"CPUPerc":"{:.2}%","MemUsage":"{:.1}MiB / {:.1}MiB","MemPerc":"{:.2}%","NetIO":"{}/{}","BlockIO":"{}/{}","PIDs":{}}}"#,
        json::quote_string(id),
        runtime_label,
        json::quote_string(name),
        json::quote_string(uptime),
        s.cpu_perc,
        bytes_to_mib(s.mem_usage),
        bytes_to_mib(s.mem_limit),
        s.mem_perc,
        human_bytes(s.net_rx),
        human_bytes(s.net_tx),
        human_bytes(s.block_read),
        human_bytes(s.block_write),
        s.pids,
    );
}

fn bytes_to_mib(b: u64) -> f64 {
    (b as f64) / 1024.0 / 1024.0
}

fn human_bytes(b: u64) -> String {
    // docker uses kB / MB / GB / TB / PB (decimal-ish; actual is 1024-based
    // for kB then 1000-based for higher units). We follow the same scheme.
    const KB: f64 = 1024.0;
    const MB: f64 = 1000.0 * KB;
    const GB: f64 = 1000.0 * MB;
    const TB: f64 = 1000.0 * GB;
    const PB: f64 = 1000.0 * TB;
    let v = b as f64;
    if v < KB {
        format!("{:.1}B", v)
    } else if v < MB {
        format!("{:.1}kB", v / KB)
    } else if v < GB {
        format!("{:.1}MB", v / MB)
    } else if v < TB {
        format!("{:.1}GB", v / GB)
    } else if v < PB {
        format!("{:.1}TB", v / TB)
    } else {
        format!("{:.1}PB", v / PB)
    }
}

pub fn parse_pm2_response(buf: &[u8]) -> io::Result<&[u8]> {
    if buf.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty response"));
    }
    let version_argc = buf[0];
    let argc = version_argc & 0x0f;
    if argc < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid AMP argc"));
    }

    let mut offset = 1;
    // Read Arg 0 (payload)
    if offset + 4 > buf.len() { return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated payload len")); }
    let len0 = u32::from_be_bytes([buf[offset], buf[offset+1], buf[offset+2], buf[offset+3]]) as usize;
    offset += 4;
    if offset + len0 > buf.len() { return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated payload data")); }
    
    let data = &buf[offset..offset+len0];
    if data.starts_with(b"j:") {
        Ok(&data[2..])
    } else {
        Ok(data)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pm2Process {
    pub pid: u64,
    pub pm_id: u64,
    pub name: String,
    pub status: String,
    pub pm_uptime: u64,
    pub memory: u64,
    pub cpu: f64,
}

pub fn parse_pm2_list(body: &[u8]) -> io::Result<Vec<Pm2Process>> {
    let root = match json::classify(body, 0) {
        Some(s) if s.kind == json::Kind::Array => s,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "pm2: not an array")),
    };

    let mut out = Vec::new();
    json::array_iter(body, root, |item| {
        if item.kind != json::Kind::Object {
            return true;
        }

        let pid = json::object_get(body, item, "pid")
            .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
            .unwrap_or(0);

        let name = json::object_get(body, item, "name")
            .and_then(|s| json::as_str(body, s).map(str::to_owned))
            .unwrap_or_default();

        let mut pm_id = 0;
        let mut status = String::new();
        let mut pm_uptime = 0;

        if let Some(env) = json::object_get(body, item, "pm2_env") {
            if env.kind == json::Kind::Object {
                pm_id = json::object_get(body, env, "pm_id")
                    .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
                    .unwrap_or(0);
                status = json::object_get(body, env, "status")
                    .and_then(|s| json::as_str(body, s).map(str::to_owned))
                    .unwrap_or_default();
                pm_uptime = json::object_get(body, env, "pm_uptime")
                    .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
                    .unwrap_or(0);
            }
        }

        let mut memory = 0;
        let mut cpu = 0.0;
        if let Some(monit) = json::object_get(body, item, "monit") {
            if monit.kind == json::Kind::Object {
                memory = json::object_get(body, monit, "memory")
                    .and_then(|s| json::parse_f64(&body[s.start..s.end]).map(|v| v as u64))
                    .unwrap_or(0);
                cpu = json::object_get(body, monit, "cpu")
                    .and_then(|s| json::parse_f64(&body[s.start..s.end]))
                    .unwrap_or(0.0);
            }
        }

        out.push(Pm2Process {
            pid,
            pm_id,
            name,
            status,
            pm_uptime,
            memory,
            cpu,
        });
        true
    });

    Ok(out)
}

pub fn append_pm2_json(
    out: &mut String,
    p: &Pm2Process,
    host_mem: u64,
) {
    use std::fmt::Write;

    let mut uptime_str = p.status.clone();
    if p.status == "online" && p.pm_uptime > 0 {
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            let now_ms = now.as_millis() as u64;
            if now_ms > p.pm_uptime {
                let diff_secs = (now_ms - p.pm_uptime) / 1000;
                let days = diff_secs / 86400;
                let hours = (diff_secs % 86400) / 3600;
                let mins = (diff_secs % 3600) / 60;
                if days > 0 {
                    uptime_str = format!("Up {} days", days);
                } else if hours > 0 {
                    uptime_str = format!("Up {} hours", hours);
                } else if mins > 0 {
                    uptime_str = format!("Up {} minutes", mins);
                } else {
                    uptime_str = format!("Up {} seconds", diff_secs % 60);
                }
            }
        }
    }

    let mem_perc = if host_mem > 0 {
        (p.memory as f64) * 100.0 / (host_mem as f64)
    } else {
        0.0
    };

    let id_str = format!("{}", p.pid);

    let _ = write!(
        out,
        r#"{{"ID":{},"Runtime":"pm2","Name":{},"Uptime":{},"CPUPerc":"{:.2}%","MemUsage":"{:.1}MiB / {:.1}MiB","MemPerc":"{:.2}%","NetIO":"N/A","BlockIO":"N/A","PIDs":1}}"#,
        json::quote_string(&id_str),
        json::quote_string(&p.name),
        json::quote_string(&uptime_str),
        p.cpu,
        bytes_to_mib(p.memory),
        bytes_to_mib(host_mem),
        mem_perc,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_simple_containers() {
        let body: &[u8] = br#"[{"Id":"abc123def456abc123def456abc123def456abc123def456abc123def456abc1","Names":["/foo","/foo2"],"Image":"x:latest"},{"Id":"789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789g","Names":["/bar"],"Image":"y:latest"}]"#;
        let r = parse_container_list(body).unwrap();
        assert_eq!(r.len(), 2, "want 2 got {:?}", r);
        assert_eq!(r[0].0, "abc123def456abc123def456abc123def456abc123def456abc123def456abc1");
        assert_eq!(r[0].1, "foo");
        assert_eq!(r[0].2, "");
        assert_eq!(r[1].0, "789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789ghi789g");
        assert_eq!(r[1].1, "bar");
        assert_eq!(r[1].2, "");
    }

    #[test]
    fn parses_real_daemon_head() {
        let body: &[u8] = br#"[{"Id":"e1e5e714b23abce23d54d664ff709bdfb9794c84e88ae49842b0dde221f2dea5","Names":["/koburi"],"Image":"ghcr.io/scathachgrip/koburi:latest","ImageID":"sha256:5f7cdc48afec65c53313d59bb0b6682cc5e6ef54147f85bcab44514bbdc78fcc","ImageManifestDescriptor":{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:5f7cdc48afec65c53313d59bb0b6682cc5e6ef54147f85bcab44514bbdc78fcc","size":274}}]"#;
        let r = parse_container_list(body).unwrap();
        assert_eq!(r.len(), 1, "want 1, got {:?}", r);
        assert_eq!(r[0].0, "e1e5e714b23abce23d54d664ff709bdfb9794c84e88ae49842b0dde221f2dea5");
        assert_eq!(r[0].1, "koburi");
        assert_eq!(r[0].2, "");
    }

    #[test]
    fn parses_real_daemon_head_with_nested_network() {
        let body: &[u8] = br#"[{"Id":"aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44","Names":["/app"],"Image":"x","ImageID":"sha256:aaa","ImageManifestDescriptor":{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:aaa","size":1,"platform":{"architecture":"amd64","os":"linux"}},"Ports":[{"IP":"::","PrivatePort":8001,"PublicPort":8001,"Type":"tcp"}],"Labels":{"x":"y"},"State":"running","Status":"Up 11 days","HostConfig":{"NetworkMode":"d"},"Health":{"Status":"none","FailingStreak":0},"NetworkSettings":{"Networks":{"d":{"IPAMConfig":null,"Links":null,"Aliases":null,"DriverOpts":null,"GwPriority":0,"NetworkID":"n","EndpointID":"e","Gateway":"172.18.0.1","IPAddress":"172.18.0.2","MacAddress":"aa:bb:cc:dd:ee:ff","IPPrefixLen":16,"IPv6Gateway":"","GlobalIPv6Address":"","GlobalIPv6PrefixLen":0,"DNSNames":null}}},"Mounts":[]}]"#;
        let r = parse_container_list(body).unwrap();
        assert_eq!(r.len(), 1, "want 1, got {:?}", r);
        assert_eq!(r[0].0, "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44");
        assert_eq!(r[0].1, "app");
        assert_eq!(r[0].2, "Up 11 days");
    }

    #[test]
    fn parses_empty_array() {
        let r = parse_container_list(b"[]").unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn parses_daemon_error_message() {
        let body: &[u8] = br#"{"message":"page not found"}"#;
        let err = parse_container_list(body).unwrap_err();
        assert_eq!(err.to_string(), "page not found");

        let err2 = compute(body).unwrap_err();
        assert_eq!(err2.to_string(), "page not found");
    }

    #[test]
    fn parses_pm2_jlist() {
        let body = br#"[{"pid":39535,"name":"homepage","pm2_env":{"name":"homepage","status":"online","pm_uptime":1629720000000,"pm_id":0},"monit":{"memory":90554368,"cpu":0.5}}]"#;
        let r = parse_pm2_list(body).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].pid, 39535);
        assert_eq!(r[0].pm_id, 0);
        assert_eq!(r[0].name, "homepage");
        assert_eq!(r[0].status, "online");
        assert_eq!(r[0].pm_uptime, 1629720000000);
        assert_eq!(r[0].memory, 90554368);
        assert_eq!(r[0].cpu, 0.5);
    }

    #[test]
    fn parses_pm2_response_bytes() {
        let mut resp = Vec::new();
        resp.push(0x12); // version 1, argc = 2
        
        // Arg 0: j:{"args":[[{"pid": 123}]]}
        let payload = b"j:{\"args\":[[{\"pid\": 123}]]}";
        resp.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        resp.extend_from_slice(payload);

        // Arg 1: "1"
        resp.extend_from_slice(&1u32.to_be_bytes());
        resp.extend_from_slice(b"1");

        let data = parse_pm2_response(&resp).unwrap();
        assert_eq!(data, b"{\"args\":[[{\"pid\": 123}]]}");
    }
}
