use crate::latency;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    env,
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const IP: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
const HOST: &str = "speed.cloudflare.com";
const DOWN_BYTES: usize = 10_000_000;
const UP_BYTES: usize = 4_000_000;

#[derive(Clone, Copy, Debug, Serialize)]
enum Failure {
    Start,
    Dns,
    Connect,
    Timeout,
    Tls,
    Curl(i32),
    Http(u16),
    Invalid,
    Incomplete,
}

fn system_command(exe: &str) -> Command {
    let root = env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let mut command = Command::new(PathBuf::from(root).join("System32").join(exe));
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

pub(crate) fn powershell(script: &str) -> Command {
    let mut command = system_command(r"WindowsPowerShell\v1.0\powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    command
}

pub fn diagnose(stage: &impl Fn(usize)) -> Value {
    stage(1);
    let gateway = latency::gateway(IP);
    let local = gateway.and_then(latency::baseline);
    stage(2);
    let (ip, dns, https, ping) = thread::scope(|scope| {
        let ip = scope.spawn(|| {
            TcpStream::connect_timeout(&SocketAddr::from((IP, 443)), Duration::from_secs(3)).is_ok()
        });
        let dns = scope.spawn(|| {
            (HOST, 443)
                .to_socket_addrs()
                .is_ok_and(|mut addresses| addresses.next().is_some())
        });
        let ping = scope.spawn(|| latency::baseline(IP));
        let https = curl(&format!("https://{HOST}/__down?bytes=0"), None, "8");
        (
            ip.join().unwrap(),
            dns.join().unwrap(),
            https,
            ping.join().unwrap(),
        )
    });
    json!({
        "kind": "diagnose",
        "ip": ip,
        "dns": dns,
        "https": https.map(|transfer| transfer.first_byte_ms),
        "ping": ping,
        "gateway": gateway,
        "local": local,
    })
}

pub fn full(stage: &impl Fn(usize)) -> Value {
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|time| time.as_millis());
    let diagnosis = diagnose(stage);
    let speed = speed(stage, true);
    json!({ "kind": "full", "at": at, "diagnosis": diagnosis, "speed": speed })
}

pub fn speed(stage: &impl Fn(usize), already_has_baseline: bool) -> Value {
    let idle = if already_has_baseline {
        None
    } else {
        stage(1);
        latency::baseline(IP)
    };
    let download_step = if already_has_baseline { 3 } else { 2 };
    stage(download_step);
    let download = measure(DOWN_BYTES, |bytes| {
        under_load(&format!("https://{HOST}/__down?bytes={bytes}"), None)
    });
    stage(download_step + 1);
    let upload = measure(UP_BYTES, |bytes| {
        under_load(&format!("https://{HOST}/__up"), Some(bytes))
    });
    json!({ "kind": "speed", "download": download, "upload": upload, "idle": idle })
}

#[derive(Debug, Serialize)]
struct SpeedReading {
    mbps: f64,
    ping: Option<f64>,
}

fn measure(
    bytes: usize,
    transfer: impl Fn(usize) -> Result<Transfer, Failure>,
) -> Result<SpeedReading, Failure> {
    const PROBE_BYTES: usize = 500_000;
    let probe = transfer(PROBE_BYTES)?;
    let larger_sample_fits = bytes as f64 / probe.bytes_per_second < 12.0;
    let estimate = probe.reading(PROBE_BYTES)?;
    if larger_sample_fits {
        return transfer(bytes)
            .and_then(|sample| sample.reading(bytes))
            .or(Ok(estimate));
    }
    Ok(estimate)
}

fn under_load(url: &str, upload: Option<usize>) -> Result<Transfer, Failure> {
    let start = Instant::now();
    let done = AtomicBool::new(false);
    let (transfer, samples) = thread::scope(|scope| {
        let probe = scope.spawn(|| {
            let mut samples = Vec::new();
            let Some(echo) = latency::Echo::open() else {
                return samples;
            };
            while !done.load(Ordering::Relaxed) {
                let sent = start.elapsed().as_secs_f64();
                if let Some(ms) = echo.ping(IP) {
                    samples.push((sent, start.elapsed().as_secs_f64(), ms));
                }
                thread::sleep(Duration::from_millis(85));
            }
            samples
        });
        let transfer = curl(url, upload, "20");
        done.store(true, Ordering::Relaxed);
        (transfer, probe.join().unwrap())
    });
    let mut transfer = transfer?;
    if let Some((begin, end)) = transfer.window(upload.is_some()) {
        transfer.ping = loaded_ping(&samples, begin, end);
    }
    Ok(transfer)
}

fn loaded_ping(samples: &[(f64, f64, u32)], start: f64, end: f64) -> Option<f64> {
    // Leave a margin for curl process startup and keep only round trips within the transfer.
    let mut replies: Vec<_> = samples
        .iter()
        .filter(|&&(sent, received, _)| sent >= start + 0.035 && received <= end - 0.035)
        .map(|&(_, _, ms)| ms)
        .collect();
    if replies.len() < 2 {
        return None;
    }
    latency::median(&mut replies)
}

pub fn repair() -> Value {
    let result =
        powershell("try { Clear-DnsClientCache -ErrorAction Stop; exit 0 } catch { exit 1 }")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
    json!({ "kind": "repair", "success": result })
}

fn curl(url: &str, upload: Option<usize>, timeout: &str) -> Result<Transfer, Failure> {
    let (size, speed) = if upload.is_some() {
        ("%{size_upload}", "%{speed_upload}")
    } else {
        ("%{size_download}", "%{speed_download}")
    };
    let mut command = system_command("curl.exe");
    command.args([
        "-q", "--silent", "--noproxy", "*", "--connect-timeout", "5", "--max-time", timeout,
        "--max-filesize", "10000000", "--output", "NUL", "--write-out",
        &format!("%{{http_code}} {size} {speed} %{{time_starttransfer}} %{{time_pretransfer}} %{{time_total}} %{{time_posttransfer}}"),
    ]);
    if upload.is_some() {
        command.args([
            "--header",
            "Content-Type: application/octet-stream",
            "--data-binary",
            "@-",
        ]);
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Failure::Start)?;
    let sent = if let Some(bytes) = upload {
        child.stdin.take().unwrap().write_all(&vec![0; bytes])
    } else {
        Ok(())
    };
    let output = child.wait_with_output().map_err(|_| Failure::Incomplete)?;
    if !output.status.success() {
        return Err(match output.status.code().unwrap_or(-1) {
            6 => Failure::Dns,
            7 => Failure::Connect,
            28 => Failure::Timeout,
            35 | 60 => Failure::Tls,
            code => Failure::Curl(code),
        });
    }
    sent.map_err(|_| Failure::Incomplete)?;
    Transfer::parse(&String::from_utf8_lossy(&output.stdout))
}

struct Transfer {
    bytes: usize,
    bytes_per_second: f64,
    first_byte_ms: f64,
    pretransfer: Option<f64>,
    total: Option<f64>,
    posttransfer: Option<f64>,
    ping: Option<f64>,
}

impl Transfer {
    fn parse(output: &str) -> Result<Self, Failure> {
        let parts: Vec<_> = output.split_whitespace().collect();
        let [code, bytes, rate, elapsed, timing @ ..] = parts.as_slice() else {
            return Err(Failure::Invalid);
        };
        let code: u16 = code.parse().map_err(|_| Failure::Invalid)?;
        if !(200..300).contains(&code) {
            return Err(Failure::Http(code));
        }
        let bytes = bytes.parse().map_err(|_| Failure::Invalid)?;
        let bytes_per_second: f64 = rate.parse().map_err(|_| Failure::Invalid)?;
        let seconds: f64 = elapsed.parse().map_err(|_| Failure::Invalid)?;
        if !bytes_per_second.is_finite()
            || bytes_per_second < 0.0
            || !seconds.is_finite()
            || seconds < 0.0
        {
            return Err(Failure::Invalid);
        }
        Ok(Self {
            bytes,
            bytes_per_second,
            first_byte_ms: seconds * 1000.0,
            pretransfer: time_field(timing, 0),
            total: time_field(timing, 1),
            posttransfer: time_field(timing, 2),
            ping: None,
        })
    }

    fn reading(self, expected: usize) -> Result<SpeedReading, Failure> {
        if self.bytes != expected || self.bytes_per_second == 0.0 {
            return Err(Failure::Incomplete);
        }
        Ok(SpeedReading {
            mbps: self.bytes_per_second * 8.0 / 1_000_000.0,
            ping: self.ping,
        })
    }

    fn window(&self, upload: bool) -> Option<(f64, f64)> {
        if upload {
            Some((self.pretransfer?, self.posttransfer?))
        } else {
            Some((self.first_byte_ms / 1000.0, self.total?))
        }
    }
}

fn time_field(fields: &[&str], index: usize) -> Option<f64> {
    fields
        .get(index)?
        .parse::<f64>()
        .ok()
        .filter(|time| time.is_finite() && *time >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::{Transfer, loaded_ping, measure};

    #[test]
    fn converts_curl_speed_from_bytes_to_megabits() {
        let result = Transfer::parse("200 10000000 1250000 0.045").unwrap();
        assert_eq!(result.first_byte_ms, 45.0);
        assert_eq!(result.reading(10_000_000).unwrap().mbps, 10.0);
    }

    #[test]
    fn rejects_incomplete_transfers_and_invalid_curl_output() {
        assert!(
            Transfer::parse("200 1500 1250 0.045")
                .unwrap()
                .reading(2000)
                .is_err()
        );
        assert!(Transfer::parse("302 2000 1250 0.045").is_err());
        assert!(Transfer::parse("200 2000 NaN 0.045").is_err());
        assert!(Transfer::parse("200 2000 1250").is_err());
    }

    #[test]
    fn speed_uses_the_probe_on_a_slow_connection() {
        let result = measure(10_000_000, |bytes| {
            assert_eq!(bytes, 500_000);
            Transfer::parse("200 500000 125000 0.08")
        });
        assert_eq!(result.unwrap().mbps, 1.0);
    }

    #[test]
    fn speed_uses_a_larger_sample_when_it_can_finish() {
        let result = measure(10_000_000, |bytes| {
            let rate = if bytes == 500_000 {
                1_250_000
            } else {
                2_500_000
            };
            Transfer::parse(&format!("200 {bytes} {rate} 0.08"))
        });
        assert_eq!(result.unwrap().mbps, 20.0);
    }

    #[test]
    fn loaded_ping_ignores_round_trips_outside_the_actual_transfer() {
        let samples = [
            (0.11, 0.20, 90),
            (0.15, 0.20, 50),
            (0.30, 0.34, 40),
            (0.43, 0.48, 50),
        ];
        assert_eq!(loaded_ping(&samples, 0.10, 0.47), Some(45.0));
        assert_eq!(loaded_ping(&samples, 0.29, 0.47), None);
        let reading = Transfer::parse("200 1500 1250 0.23 0.12 0.75 0.19").unwrap();
        assert_eq!(reading.window(true), Some((0.12, 0.19)));
        assert_eq!(reading.window(false), Some((0.23, 0.75)));
        assert!(
            Transfer::parse("200 1500 1250 0.23 0.12 0.75")
                .unwrap()
                .window(true)
                .is_none()
        );
    }
}
