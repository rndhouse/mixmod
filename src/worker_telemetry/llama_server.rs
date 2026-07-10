//! llama-server telemetry collection.

use std::env;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;

use super::{WorkerBackendSlotTelemetry, WorkerBackendTelemetry};

const PROVIDER: &str = "llama_server";
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const IO_TIMEOUT: Duration = Duration::from_millis(700);
const MAX_RESPONSE_BYTES: usize = 1_048_576;

/// Collect llama-server telemetry for a local OpenCode worker.
pub(crate) fn collect_for_opencode_worker(provider: &str) -> Option<WorkerBackendTelemetry> {
    let explicit_base = env::var("MIXMOD_LLAMA_TELEMETRY_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let should_probe = explicit_base.is_some() || provider_looks_like_llama_server(provider);
    if !should_probe {
        return None;
    }

    let Some(base_url) = explicit_base.or_else(|| {
        env::var("MIXMOD_OPENCODE_BASE_URL")
            .ok()
            .and_then(|value| llama_server_root_from_openai_base(&value))
    }) else {
        return Some(unavailable("missing llama-server telemetry base URL"));
    };

    Some(collect_from_base_url(&base_url))
}

/// Collect telemetry from a llama-server root URL.
pub(crate) fn collect_from_base_url(base_url: &str) -> WorkerBackendTelemetry {
    let captured_at = Utc::now().to_rfc3339();
    let slots = fetch_http(base_url, "/slots").and_then(|body| parse_slots(&body));
    let metrics = fetch_http(base_url, "/metrics").map(|body| parse_metrics(&body));
    let slots_ok = slots.is_ok();
    let metrics_ok = metrics.is_ok();

    if !slots_ok && !metrics_ok {
        let error = format!(
            "slots: {}; metrics: {}",
            slots
                .as_ref()
                .err()
                .map(|error| truncate_error(error))
                .unwrap_or("unavailable".to_string()),
            metrics
                .as_ref()
                .err()
                .map(|error| truncate_error(error))
                .unwrap_or("unavailable".to_string())
        );
        return WorkerBackendTelemetry {
            provider: PROVIDER.to_string(),
            available: false,
            captured_at,
            error: Some(error),
            ..WorkerBackendTelemetry::default()
        };
    }

    let slot_sample = slots.unwrap_or_default();
    let metric_sample = metrics.unwrap_or_default();
    WorkerBackendTelemetry {
        provider: PROVIDER.to_string(),
        available: true,
        captured_at,
        ctx_size: slot_sample.ctx_size,
        requests_processing: metric_sample.requests_processing,
        requests_deferred: metric_sample.requests_deferred,
        tokens_max_observed: metric_sample.tokens_max_observed,
        active_slots: slot_sample.active_slots,
        error: None,
    }
}

fn unavailable(error: &str) -> WorkerBackendTelemetry {
    WorkerBackendTelemetry {
        provider: PROVIDER.to_string(),
        available: false,
        captured_at: Utc::now().to_rfc3339(),
        error: Some(error.to_string()),
        ..WorkerBackendTelemetry::default()
    }
}

fn provider_looks_like_llama_server(provider: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    provider.contains("llama") || provider.contains("llama.cpp")
}

fn llama_server_root_from_openai_base(base_url: &str) -> Option<String> {
    http_origin(base_url.trim())
}

fn http_origin(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split('/').next()?.trim();
    if authority.is_empty() {
        return None;
    }
    Some(format!("http://{authority}"))
}

fn fetch_http(base_url: &str, path: &str) -> Result<String, String> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let target = HttpTarget::parse(&url)?;
    let mut addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|error| format!("resolve failed: {error}"))?;
    let addr = addrs
        .next()
        .ok_or_else(|| "resolve failed: no socket addresses".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|error| format!("connect failed: {error}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set read timeout failed: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set write timeout failed: {error}"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        target.path, target.host_header
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write failed: {error}"))?;

    let mut response = Vec::new();
    let mut limited = stream.take(MAX_RESPONSE_BYTES as u64);
    limited
        .read_to_end(&mut response)
        .map_err(|error| format!("read failed: {error}"))?;
    parse_http_response(&response)
}

#[derive(Debug)]
struct HttpTarget {
    host: String,
    port: u16,
    host_header: String,
    path: String,
}

impl HttpTarget {
    fn parse(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| "only http:// llama-server telemetry URLs are supported".to_string())?;
        let (authority, path) = rest
            .split_once('/')
            .map(|(authority, path)| (authority, format!("/{path}")))
            .unwrap_or((rest, "/".to_string()));
        let authority = authority.trim();
        if authority.is_empty() {
            return Err("missing host".to_string());
        }
        let (host, port) = parse_authority(authority)?;
        Ok(Self {
            host,
            port,
            host_header: authority.to_string(),
            path,
        })
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16), String> {
    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.contains(']')
    {
        let port = port
            .parse::<u16>()
            .map_err(|error| format!("invalid port: {error}"))?;
        return Ok((host.to_string(), port));
    }
    Ok((authority.to_string(), 80))
}

fn parse_http_response(response: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(response);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "missing HTTP status".to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    Ok(body.to_string())
}

#[derive(Default)]
struct SlotSample {
    ctx_size: Option<u64>,
    active_slots: Vec<WorkerBackendSlotTelemetry>,
}

fn parse_slots(body: &str) -> Result<SlotSample, String> {
    let value = serde_json::from_str::<Value>(body).map_err(|error| error.to_string())?;
    let slots = value
        .as_array()
        .ok_or_else(|| "slots response was not an array".to_string())?;
    let ctx_size = slots
        .iter()
        .filter_map(|slot| slot.get("n_ctx").and_then(Value::as_u64))
        .max();
    let active_slots = slots
        .iter()
        .filter(|slot| {
            slot.get("is_processing")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(parse_active_slot)
        .collect();
    Ok(SlotSample {
        ctx_size,
        active_slots,
    })
}

fn parse_active_slot(slot: &Value) -> Option<WorkerBackendSlotTelemetry> {
    let id = slot.get("id").and_then(Value::as_u64)?;
    Some(WorkerBackendSlotTelemetry {
        id,
        ctx_size: slot.get("n_ctx").and_then(Value::as_u64),
        is_processing: slot
            .get("is_processing")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        decoded_tokens: slot
            .pointer("/next_token/n_decoded")
            .and_then(Value::as_u64),
        remaining_tokens: slot.pointer("/next_token/n_remain").and_then(Value::as_i64),
    })
}

#[derive(Default)]
struct MetricSample {
    requests_processing: Option<u64>,
    requests_deferred: Option<u64>,
    tokens_max_observed: Option<u64>,
}

fn parse_metrics(body: &str) -> MetricSample {
    let mut sample = MetricSample::default();
    for line in body.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if metric_matches(line, "llamacpp:requests_processing") {
            sample.requests_processing = parse_metric_u64(line);
        } else if metric_matches(line, "llamacpp:requests_deferred") {
            sample.requests_deferred = parse_metric_u64(line);
        } else if metric_matches(line, "llamacpp:n_tokens_max") {
            sample.tokens_max_observed = parse_metric_u64(line);
        }
    }
    sample
}

fn metric_matches(line: &str, metric: &str) -> bool {
    line.starts_with(metric)
        && line[metric.len()..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || ch == '{')
}

fn parse_metric_u64(line: &str) -> Option<u64> {
    let value = line.split_whitespace().last()?;
    let value = value.parse::<f64>().ok()?;
    if value.is_finite() && value >= 0.0 {
        Some(value as u64)
    } else {
        None
    }
}

fn truncate_error(error: &str) -> String {
    if error.chars().count() <= 120 {
        return error.to_string();
    }
    let mut truncated = error.chars().take(120).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_llama_server_root_from_openai_base_url() {
        assert_eq!(
            llama_server_root_from_openai_base("http://127.0.0.1:11434/v1"),
            Some("http://127.0.0.1:11434".to_string())
        );
    }

    #[test]
    fn parses_slots_into_active_raw_fields() {
        let sample = parse_slots(
            r#"[
              {"id":0,"n_ctx":32768,"is_processing":false},
              {"id":1,"n_ctx":32768,"is_processing":true,
               "next_token":{"n_decoded":814,"n_remain":-1}}
            ]"#,
        )
        .unwrap();

        assert_eq!(sample.ctx_size, Some(32768));
        assert_eq!(
            sample.active_slots,
            vec![WorkerBackendSlotTelemetry {
                id: 1,
                ctx_size: Some(32768),
                is_processing: true,
                decoded_tokens: Some(814),
                remaining_tokens: Some(-1),
            }]
        );
    }

    #[test]
    fn parses_prometheus_metrics_into_raw_fields() {
        let sample = parse_metrics(
            r#"
# HELP llamacpp:n_tokens_max High watermark
llamacpp:requests_processing 1
llamacpp:requests_deferred{model="qwen"} 2
llamacpp:n_tokens_max 27142
"#,
        );

        assert_eq!(sample.requests_processing, Some(1));
        assert_eq!(sample.requests_deferred, Some(2));
        assert_eq!(sample.tokens_max_observed, Some(27142));
    }
}
