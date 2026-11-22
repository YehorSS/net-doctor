# NET DOCTOR

NET DOCTOR is a Windows desktop app for diagnosing a slow or unstable internet connection. It measures the route to your router and to the internet separately, then checks what happens to latency while data is being downloaded or uploaded.

[Download for Windows](https://github.com/YehorSS/net-doctor/releases/latest)

## What it does

- **Find the cause** measures IPv4 gateway and internet latency, packet loss, DNS and HTTPS connectivity, download and upload speed, and ping during both transfers.
- **Improve connection** tries an adjustment when the measurements support it. It can clear the DNS cache, offer a faster DNS resolver with Windows approval, or test a Wi-Fi power setting while plugged in. The app restores an unsuccessful Wi-Fi adjustment and provides a restore action for settings it keeps.
- **Before and after** compares latency, packet loss, and throughput against a saved baseline. Measurements and the English or Russian language preference are stored locally.

Download and upload tests use Cloudflare and send up to 15 MB per run, or up to 30 MB for a before-and-after run. An adjustment can help when the bottleneck is within its scope. Your ISP, router, and destination still determine much of the result.

## Run and build

The release contains a Windows x64 executable. Open it with Microsoft Edge WebView2 Runtime installed. The app uses the Windows system `curl.exe` for transfers. Upload latency requires curl 8.10 or later and enough ping replies during the transfer.

To build on Windows, install the Rust MSVC toolchain and Visual Studio Build Tools with C++ support.

```powershell
cargo test --locked
cargo build --release --locked
```

The executable is at `target\release\net-doctor.exe`. Rust handles the measurements and Windows integration. The WebView2 interface is embedded in the executable.

## По-русски

NET DOCTOR проверяет пинг до роутера и интернета, потери пакетов, DNS, HTTPS и скорость под нагрузкой. «Улучшить соединение» подбирает доступную настройку и повторяет замер. Результаты видны в таблице «До и после». Язык переключается кнопками **RU / EN**.
