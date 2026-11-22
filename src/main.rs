#![windows_subsystem = "windows"]

mod dns;
mod improve;
mod latency;
mod network;
mod wifi;

use std::{borrow::Cow, error::Error, iter::once, ptr::null_mut, thread};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use wry::{NewWindowResponse, WebViewBuilder, http::Response};

const APP_URL: &str = "http://net-doctor.localhost/";
const BUNDLED_URL: &str = "net-doctor://localhost/";

enum AppEvent {
    Run(Job),
    Stage(usize),
    Result(serde_json::Value, Option<dns::Candidate>),
}

#[derive(Clone, Copy)]
enum Job {
    Full,
    Improve,
    Diagnose,
    Speed,
    Repair,
    Optimize,
    Apply,
    Restore,
    RestoreWifi,
}

fn main() {
    if let Err(error) = run() {
        let message: Vec<u16> = format!(
            "Не удалось открыть NET DOCTOR. Проверьте наличие Microsoft Edge WebView2 Runtime.\n\nNET DOCTOR could not start. Check that Microsoft Edge WebView2 Runtime is installed.\n\n{error}"
        )
        .encode_utf16()
        .chain(once(0))
        .collect();
        let title: Vec<u16> = "NET DOCTOR".encode_utf16().chain(once(0)).collect();
        // The two NUL-terminated UTF-16 buffers outlive the modal call.
        unsafe { MessageBoxW(null_mut(), message.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let events = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("NET DOCTOR")
        .with_inner_size(LogicalSize::new(1040, 750))
        .with_min_inner_size(LogicalSize::new(480, 450))
        .build(&events)?;
    let proxy = events.create_proxy();
    let ipc = proxy.clone();
    let html = include_str!("ui/index.html")
        .replace(
            "/* APP_SKULL */",
            &include_str!("ui/skull.txt")
                .replace('&', "&amp;")
                .replace('<', "&lt;"),
        )
        .replace("/* APP_STYLES */", include_str!("ui/style.css"))
        .replace("/* APP_STRINGS */", include_str!("ui/strings.js"))
        .replace("/* APP_SCRIPT */", include_str!("ui/app.js"))
        .into_bytes();
    let webview = WebViewBuilder::new()
        .with_background_color((6, 8, 7, 255))
        .with_custom_protocol("net-doctor".into(), move |_, request| {
            if request.uri() == BUNDLED_URL {
                Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Owned(html.clone()))
                    .unwrap()
            } else {
                Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&[][..]))
                    .unwrap()
            }
        })
        .with_url(BUNDLED_URL)
        .with_initialization_script(format!(
            "window.canRestore = {}; window.canRestoreWifi = {};",
            dns::can_restore(),
            wifi::can_restore()
        ))
        .with_navigation_handler(|url| url == APP_URL)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_ipc_handler(move |request| {
            if request.uri() != APP_URL {
                return;
            }
            let job = match request.body().as_str() {
                "full" => Job::Full,
                "improve" => Job::Improve,
                "diagnose" => Job::Diagnose,
                "speed" => Job::Speed,
                "repair" => Job::Repair,
                "optimize" => Job::Optimize,
                "apply" => Job::Apply,
                "restore" => Job::Restore,
                "restoreWifi" => Job::RestoreWifi,
                _ => return,
            };
            let _ = ipc.send_event(AppEvent::Run(job));
        })
        .build(&window)?;

    let mut busy = false;
    let mut candidate = None;
    events.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } if window_id == window.id() => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(AppEvent::Run(job)) if !busy => {
                busy = true;
                let reply = proxy.clone();
                let selected = if matches!(job, Job::Apply | Job::Optimize | Job::Restore | Job::Improve) {
                    candidate.take()
                } else {
                    None
                };
                thread::spawn(move || {
                    let stage = |step| { let _ = reply.send_event(AppEvent::Stage(step)); };
                    let (result, recommendation) = match job {
                        Job::Full => (network::full(&stage), None),
                        Job::Improve => (improve::run(&stage), None),
                        Job::Diagnose => (network::diagnose(&stage), None),
                        Job::Speed => (network::speed(&stage, false), None),
                        Job::Repair => (network::repair(), None),
                        Job::Optimize => dns::benchmark(),
                        Job::Apply => (dns::apply(selected), None),
                        Job::Restore => (dns::restore(), None),
                        Job::RestoreWifi => {
                            let success = wifi::restore();
                            (serde_json::json!({
                                "kind": "restoreWifi", "success": success, "canRestoreWifi": wifi::can_restore(),
                                "message": if success { "wifiRestored" } else { "wifiRestoreError" }
                            }), None)
                        }
                    };
                    let _ = reply.send_event(AppEvent::Result(result, recommendation));
                });
            }
            Event::UserEvent(AppEvent::Stage(step)) => {
                let _ = webview.evaluate_script(&format!("window.stage({step})"));
            }
            Event::UserEvent(AppEvent::Result(result, recommendation)) => {
                busy = false;
                if let Some(recommendation) = recommendation {
                    candidate = Some(recommendation);
                }
                let _ = webview.evaluate_script(&format!("window.report({result})"));
            }
            _ => {}
        }
    });
}
