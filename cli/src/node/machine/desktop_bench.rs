//! Run on a logged-in macOS desktop with cua's Screen Recording/Accessibility
//! permissions. Captures stay in memory; only frame sizes and timings are kept.
use super::*;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "macOS desktop benchmark: NYXID_MACHINE_BENCH_CUA=/path/to/pinned/cua-driver"]
async fn macos_desktop_performance() {
    let cua = PathBuf::from(
        std::env::var("NYXID_MACHINE_BENCH_CUA").expect("set NYXID_MACHINE_BENCH_CUA"),
    );
    super::cua::verify_version(&cua).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(
        &Config {
            computer: true,
            cua_driver: Some(cua),
            roots: vec![root.path().into()],
            ..Default::default()
        },
        &Uuid::new_v4().to_string(),
        &root.path().join("node"),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let page = concat!(
        "<!doctype html><title>NyxID desktop benchmark</title><style>body{font:24px sans-serif}textarea{width:80%;height:120px}</style><textarea autofocus></textarea><main></main>",
        "<script>document.querySelector('main').innerHTML=Array.from({length:200},(_,i)=>'<p style=\"padding:20px;background:hsl('+i*19+' 60% 80%)\">Scrolling benchmark '+i+'</p>').join('')</script>"
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().fallback(move || async move { axum::response::Html(page) }),
        )
        .await
        .unwrap();
    });
    let mut browser = tokio::process::Command::new(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    )
    .args([
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-sync",
        "--window-size=1280,800",
        "--window-position=0,0",
    ])
    .arg(format!(
        "--user-data-dir={}",
        root.path().join("browser").display()
    ))
    .arg(format!("--app={url}"))
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    runtime.connect(tx, &[7; 32]).await.unwrap();
    let samples = Arc::new(Mutex::new(Vec::<(Instant, usize)>::new()));
    let dimensions = Arc::new(Mutex::new((0u32, 0u32)));
    let captured_dimensions = dimensions.clone();
    let recorded = samples.clone();
    let collector = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let crate::node::ws_client::NodeWsMessage::Binary(bytes) = message
                && nyxid_machine::binary::Frame::decode(&bytes)
                    .is_ok_and(|f| f.kind == nyxid_machine::binary::Kind::Desktop)
            {
                let frame = nyxid_machine::binary::Frame::decode(&bytes).unwrap();
                *captured_dimensions.lock().await = (
                    u32::from(u16::from_be_bytes(frame.bytes[4..6].try_into().unwrap())),
                    u32::from(u16::from_be_bytes(frame.bytes[6..8].try_into().unwrap())),
                );
                recorded.lock().await.push((Instant::now(), bytes.len()));
            }
        }
    });
    let session = Uuid::new_v4().to_string();
    runtime
        .execute(Operation::DesktopOpen, json!({"session_id":session}))
        .await
        .unwrap();
    runtime
        .execute(
            Operation::DesktopControl,
            json!({"session_id":session,"viewer_id":"benchmark","owner":true,"revision":1}),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !samples.lock().await.is_empty(),
        "macOS screen capture unavailable; enable Screen Recording and Accessibility"
    );
    let input = |tool: &str, args: Value| {
        let parameters = json!({"session_id":session,"viewer_id":"benchmark","revision":1,"tool":tool,"arguments":args});
        runtime.execute(Operation::DesktopInput, parameters)
    };
    runtime
        .owner_driver
        .as_ref()
        .unwrap()
        .tools()
        .await
        .unwrap();
    let screen = runtime
        .owner_driver
        .as_ref()
        .unwrap()
        .call("get_screen_size", json!({"session":"nyxid-owner"}))
        .await
        .unwrap();
    let (width, height) = *dimensions.lock().await;
    let point = |x: f64, y: f64| {
        json!({
            "x": x * f64::from(width) / screen["structuredContent"]["width"].as_f64().unwrap(),
            "y": y * f64::from(height) / screen["structuredContent"]["height"].as_f64().unwrap(),
        })
    };
    let scroll = |direction: &str| {
        let mut args = point(1150.0, 650.0);
        args["direction"] = json!(direction);
        args["amount"] = json!(3);
        args["by"] = json!("line");
        args
    };
    input("click", point(1150.0, 600.0)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("| Scenario | Changed frames/s | Frame bytes/s | Actions |");
    println!("|---|---:|---:|---:|");
    for scenario in ["idle", "typing", "scrolling"] {
        if scenario == "typing" {
            input("click", point(200.0, 150.0)).await.unwrap();
        }
        let start = Instant::now();
        let mut actions = 0;
        while start.elapsed() < Duration::from_secs(5) {
            match scenario {
                "typing" => {
                    input("type_text", json!({"text":"benchmark "}))
                        .await
                        .unwrap();
                }
                "scrolling" => {
                    let direction = if actions % 12 < 6 { "down" } else { "up" };
                    input("scroll", scroll(direction)).await.unwrap();
                }
                _ => {}
            }
            if scenario == "idle" {
                tokio::time::sleep(Duration::from_millis(100)).await;
            } else {
                actions += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        let data = samples.lock().await;
        let data: Vec<_> = data.iter().filter(|(at, _)| *at >= start).collect();
        println!(
            "| {scenario} | {:.2} | {:.0} | {actions} |",
            data.len() as f64 / elapsed,
            data.iter().map(|(_, n)| *n).sum::<usize>() as f64 / elapsed
        );
    }
    let mut latency = Vec::new();
    for n in 0..20 {
        let start = Instant::now();
        input("scroll", scroll(if n % 2 == 0 { "up" } else { "down" }))
            .await
            .unwrap();
        let at = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some((at, _)) = samples.lock().await.iter().find(|(at, _)| *at >= start) {
                    break *at;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        latency.push(at.duration_since(start).as_secs_f64() * 1000.0);
    }
    latency.sort_by(f64::total_cmp);
    println!(
        "macOS input-to-frame: p50={:.2}ms p95={:.2}ms",
        latency[9], latency[18]
    );
    runtime.shutdown().await;
    browser.kill().await.unwrap();
    collector.abort();
    server.abort();
}
