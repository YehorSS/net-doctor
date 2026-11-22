use crate::{dns, network, wifi};
use serde_json::{Value, json};

pub fn run(stage: &impl Fn(usize)) -> Value {
    stage(1);
    let before = network::full(&|_| {});
    let diagnosis = &before["diagnosis"];
    stage(2);

    let action;
    let after;
    if diagnosis["ip"] == true && diagnosis["dns"] == false {
        if network::repair()["success"] == true {
            stage(3);
            let checked = network::full(&|_| {});
            action = if checked["diagnosis"]["dns"] == true {
                "cacheRecovered"
            } else {
                "cacheRetried"
            };
            after = Some(checked);
        } else {
            action = "repairError";
            after = None;
        }
    } else if let Some(change) = wifi_candidate(diagnosis)
        .then(wifi::Change::start)
        .flatten()
    {
        stage(3);
        let checked = network::full(&|_| {});
        if wifi_improved(diagnosis, &checked["diagnosis"]) {
            action = "wifiKept";
            after = Some(checked);
        } else if change.undo() {
            action = "wifiReverted";
            after = Some(checked);
        } else {
            action = "wifiRestoreError";
            after = Some(checked);
        }
    } else if diagnosis["https"]["Ok"].is_number() {
        let (_, candidate) = dns::benchmark();
        if let Some(candidate) = candidate {
            if dns::apply(Some(candidate))["success"] == true {
                stage(3);
                let checked = network::full(&|_| {});
                action = if checked["diagnosis"]["https"]["Ok"].is_number() {
                    "dnsChanged"
                } else {
                    "dnsCheckFailed"
                };
                after = Some(checked);
            } else {
                action = "notApplied";
                after = None;
            }
        } else {
            action = "noAdjustment";
            after = None;
        }
    } else {
        action = "noAdjustment";
        after = None;
    }
    json!({
        "kind": "improve", "action": action, "before": before, "after": after,
        "canRestore": dns::can_restore(), "canRestoreWifi": wifi::can_restore()
    })
}

fn wifi_candidate(diagnosis: &Value) -> bool {
    diagnosis["local"]["median"].as_f64().is_some_and(|ms| {
        ms >= 15.0
            || diagnosis["local"]["loss"]
                .as_u64()
                .is_some_and(|loss| loss > 0)
    })
}

fn wifi_improved(before: &Value, after: &Value) -> bool {
    if !after["https"]["Ok"].is_number() || before["gateway"] != after["gateway"] {
        return false;
    }
    if let Some(public_ms) = before["ping"]["median"].as_f64() {
        let allowed = (public_ms + 10.0).max(public_ms * 1.2);
        if after["ping"]["median"]
            .as_f64()
            .is_none_or(|ms| ms > allowed)
            || after["ping"]["loss"].as_u64() > before["ping"]["loss"].as_u64()
        {
            return false;
        }
    }
    let old = &before["local"];
    let new = &after["local"];
    let Some(old_ms) = old["median"].as_f64() else {
        return false;
    };
    let Some(new_ms) = new["median"].as_f64() else {
        return false;
    };
    let (Some(old_loss), Some(new_loss)) = (old["loss"].as_u64(), new["loss"].as_u64()) else {
        return false;
    };
    (new_loss < old_loss && new_ms <= old_ms)
        || (new_loss <= old_loss && old_ms - new_ms >= 5.0 && new_ms <= old_ms * 0.8)
}

#[cfg(test)]
mod tests {
    use super::{wifi_candidate, wifi_improved};
    use serde_json::json;

    #[test]
    fn only_keeps_wifi_change_for_a_measured_gain_on_the_same_route() {
        let before =
            json!({"local":{"median":25,"loss":20},"gateway":"192.168.0.1","https":{"Ok":50}});
        let mut after = before.clone();
        assert!(wifi_candidate(&before));
        assert!(!wifi_improved(&before, &after));
        after["local"]["median"] = json!(10);
        assert!(wifi_improved(&before, &after));
        after["gateway"] = json!("192.168.2.1");
        assert!(!wifi_improved(&before, &after));
        after["gateway"] = before["gateway"].clone();
        after["ping"] = json!({"median": 120, "loss": 0});
        let mut before_with_ping = before.clone();
        before_with_ping["ping"] = json!({"median": 35, "loss": 0});
        assert!(!wifi_improved(&before_with_ping, &after));
        after["https"] = json!({"Err":"Timeout"});
        assert!(!wifi_improved(&before, &after));
    }
}
