use crate::network::powershell;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::Ipv4Addr,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
};

const SCRIPT: &str = include_str!("dns.ps1");

pub struct Candidate {
    id: String,
    server: Ipv4Addr,
}

#[derive(Deserialize)]
struct Timing {
    id: String,
    server: Ipv4Addr,
    current: f64,
    cloudflare: f64,
    automatic: bool,
}

fn backup_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var_os("LOCALAPPDATA")?)
            .join("NetDoctor")
            .join("dns-restore"),
    )
}

pub fn can_restore() -> bool {
    backup_path().is_some_and(|path| path.is_file())
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn worth_switching(current: f64, cloudflare: f64) -> bool {
    current - cloudflare >= 5.0 && cloudflare <= current * 0.8
}

pub fn benchmark() -> (Value, Option<Candidate>) {
    let command = format!("& {{ {SCRIPT}\n}} -Mode Compare");
    let output = powershell(&command).output().ok();
    let output = output.as_ref().filter(|result| result.status.success());
    let can_restore = can_restore();
    let Some(timing) =
        output.and_then(|result| serde_json::from_slice::<Timing>(&result.stdout).ok())
    else {
        let message = if output.is_some_and(|result| result.stdout.starts_with(b"unsupported")) {
            "unsupported"
        } else {
            "compareError"
        };
        return (
            json!({ "kind": "optimize", "message": message, "canRestore": can_restore }),
            None,
        );
    };
    let message = if can_restore {
        "backupExists"
    } else if !timing.automatic || !valid_id(&timing.id) {
        "manual"
    } else if worth_switching(timing.current, timing.cloudflare) {
        "faster"
    } else {
        "noGain"
    };
    let candidate = (message == "faster").then_some(Candidate {
        id: timing.id,
        server: timing.server,
    });
    (
        json!({
            "kind": "optimize", "message": message, "current": timing.current,
            "cloudflare": timing.cloudflare, "canRestore": can_restore
        }),
        candidate,
    )
}

fn run_elevated(arguments: &str) -> bool {
    let command = format!("& {{ {SCRIPT}\n}} {arguments}").replace('\'', "''");
    let wrapper = format!(
        r#"
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('{command}'))
        try {{
            $process = Start-Process -FilePath "$PSHOME\powershell.exe" -WindowStyle Hidden -Verb RunAs -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru -Wait -ErrorAction Stop
            exit $process.ExitCode
        }} catch {{ exit 1 }}
        "#
    );
    powershell(&wrapper)
        .output()
        .is_ok_and(|result| result.status.success())
}

pub fn apply(candidate: Option<Candidate>) -> Value {
    let Some(candidate) = candidate else {
        return dns_result("apply", "runCompare", false);
    };
    let Some(path) = backup_path() else {
        return dns_result("apply", "notApplied", false);
    };
    if fs::create_dir_all(path.parent().unwrap()).is_err() {
        return dns_result("apply", "notApplied", false);
    }
    let Ok(mut backup) = OpenOptions::new()
        .write(true)
        .share_mode(0)
        .create_new(true)
        .open(&path)
    else {
        return dns_result(
            "apply",
            if path.is_file() {
                "backupExists"
            } else {
                "notApplied"
            },
            false,
        );
    };
    if backup
        .write_all(candidate.id.as_bytes())
        .and_then(|()| backup.sync_all())
        .is_err()
    {
        drop(backup);
        let _ = fs::remove_file(path);
        return dns_result("apply", "notApplied", false);
    }
    let success = run_elevated(&format!(
        "-Mode Apply -Id '{}' -Expected '{}'",
        candidate.id, candidate.server
    ));
    drop(backup);
    dns_result(
        "apply",
        if success { "applyDone" } else { "notApplied" },
        success,
    )
}

pub fn restore() -> Value {
    let success = (|| {
        let path = backup_path()?;
        let backup = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .ok()?;
        let mut id = String::new();
        let mut limited = backup.take(37);
        limited.read_to_string(&mut id).ok()?;
        if !valid_id(&id) || !run_elevated(&format!("-Mode Restore -Id '{id}'")) {
            return None;
        }
        drop(limited);
        fs::remove_file(path).ok()
    })()
    .is_some();
    dns_result(
        "restore",
        if success {
            "restoreDone"
        } else {
            "notRestored"
        },
        success,
    )
}

fn dns_result(kind: &str, message: &str, success: bool) -> Value {
    json!({ "kind": kind, "message": message, "success": success, "canRestore": can_restore() })
}

#[cfg(test)]
mod tests {
    use super::{valid_id, worth_switching};

    #[test]
    fn dns_switch_requires_a_noticeable_gain() {
        assert!(worth_switching(31.0, 21.0));
        assert!(!worth_switching(100.0, 84.0));
        assert!(!worth_switching(5.0, 2.0));
    }

    #[test]
    fn only_canonical_interface_ids_can_be_used_by_the_restore_command() {
        assert!(valid_id("c984d6b2-0ec0-40af-ad44-d61e45c5ce3d"));
        assert!(!valid_id("c984d6b2-0ec0-40af-ad44-d61e45c5ce3'"));
        assert!(!valid_id("c984d6b2-0ec0-40af-ad44-d61e45c5ce3d-extra"));
    }
}
