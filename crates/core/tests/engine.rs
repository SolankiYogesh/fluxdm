//! End-to-end tests against a small, deliberately unreliable local server.

use fluxdm_core::{DownloadRequest, Manager, Settings, Status, Task};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;

#[derive(Default)]
struct Behaviour {
    ignore_range: bool,
    /// Omit Content-Length and close to mark the end.
    unknown_length: bool,
    /// Cut this many responses short.
    drops: AtomicUsize,
    /// Reply 503 above this many simultaneous transfers (0 = unlimited).
    connection_cap: usize,
    /// Like some CDNs: any request with If-Range gets the whole file.
    breaks_if_range: bool,
    /// Delay between 16 KiB blocks.
    delay_ms: u64,
}

struct Server {
    base: String,
    body: Arc<Vec<u8>>,
    etag: Mutex<String>,
    behaviour: Behaviour,
    active: AtomicUsize,
    peak: AtomicUsize,
    ranges: Mutex<Vec<u64>>,
    cookies: Mutex<Vec<(String, Option<String>)>>,
    routes: Mutex<Vec<(String, Vec<u8>)>>,
}

fn payload(len: usize) -> Vec<u8> {
    let mut x = 0x9E3779B97F4A7C15u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

async fn start(len: usize, behaviour: Behaviour) -> Arc<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Arc::new(Server {
        base: format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port()),
        body: Arc::new(payload(len)),
        etag: Mutex::new("\"v1\"".into()),
        behaviour,
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        ranges: Mutex::default(),
        cookies: Mutex::default(),
        routes: Mutex::default(),
    });
    let s = server.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(s.clone(), stream));
        }
    });
    server
}

async fn serve(s: Arc<Server>, stream: TcpStream) {
    let (read, mut out) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        if line.trim_end().is_empty() {
            break;
        }
        lines.push(line.trim_end().to_string());
    }
    let path = lines[0].split(' ').nth(1).unwrap().to_string();
    let header = |name: &str| {
        lines.iter().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    s.cookies.lock().unwrap().push((path.clone(), header("cookie")));

    if let Some(target) = path.strip_prefix("/redirect?to=") {
        let r = format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = out.write_all(r.as_bytes()).await;
        return;
    }
    if path == "/basic" && header("authorization").as_deref() != Some("Basic dTpw") {
        let _ = out
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"t\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    }
    if path == "/missing" {
        let _ = out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    let route = s.routes.lock().unwrap().iter().find(|r| r.0 == path).map(|r| r.1.clone());
    let body: Arc<Vec<u8>> = route.map(Arc::new).unwrap_or_else(|| s.body.clone());
    let total = body.len() as u64;
    let etag = s.etag.lock().unwrap().clone();

    let range = header("range").filter(|_| !s.behaviour.ignore_range).and_then(|v| {
        let (a, b) = v.strip_prefix("bytes=")?.split_once('-')?;
        let start: u64 = a.parse().ok()?;
        let end: u64 = if b.is_empty() { total } else { b.parse::<u64>().ok()? + 1 };
        Some((start, end.min(total)))
    });
    let range = range.filter(|_| {
        header("if-range").is_none_or(|v| !s.behaviour.breaks_if_range && v == etag)
    });
    if range.is_some_and(|(a, b)| a >= b) {
        let _ = out.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }

    let now = s.active.fetch_add(1, SeqCst) + 1;
    let guard = Guard(s.clone());
    if s.behaviour.connection_cap > 0 && now > s.behaviour.connection_cap {
        let _ = out.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    s.peak.fetch_max(now, SeqCst);

    let (status, start, end) = match range {
        Some((a, b)) => {
            s.ranges.lock().unwrap().push(a);
            ("206 Partial Content", a, b)
        }
        None => ("200 OK", 0, total),
    };
    let mut head = format!("HTTP/1.1 {status}\r\nETag: {etag}\r\nConnection: close\r\n");
    if !s.behaviour.unknown_length {
        head += &format!("Content-Length: {}\r\n", end - start);
    }
    if range.is_some() {
        head += &format!("Content-Range: bytes {start}-{}/{total}\r\n", end - 1);
    }
    head += "\r\n";
    if out.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let slice = &body[start as usize..end as usize];
    let drop_this = slice.len() > 64 * 1024
        && s.behaviour.drops.try_update(SeqCst, SeqCst, |n| n.checked_sub(1)).is_ok();
    let limit = if drop_this { slice.len() / 3 } else { slice.len() };
    for block in slice[..limit].chunks(16 * 1024) {
        if out.write_all(block).await.is_err() {
            break;
        }
        if s.behaviour.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(s.behaviour.delay_ms)).await;
        }
    }
    drop(guard);
    let _ = out.shutdown().await;
}

struct Guard(Arc<Server>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, SeqCst);
    }
}

fn manager(dir: &std::path::Path, change: impl FnOnce(&mut Settings)) -> Arc<Manager> {
    let m = Manager::open(dir.join("data"), Handle::current()).unwrap();
    let mut s = Settings {
        download_dir: dir.join("out"),
        category_folders: false,
        min_split: 32 * 1024,
        max_connections: 6,
        confirm_browser: false,
        ..Settings::default()
    };
    change(&mut s);
    m.update_settings(s).unwrap();
    m
}

fn task(m: &Manager, id: u64) -> Task {
    m.list().into_iter().find(|v| v.task.id == id).unwrap().task
}

async fn wait_until(m: &Manager, id: u64, what: &str, ready: impl Fn(&Task) -> bool) -> Task {
    let start = Instant::now();
    loop {
        m.tick_at(0, 0);
        let t = task(m, id);
        if ready(&t) {
            return t;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}: {t:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn finished(m: &Manager, id: u64) -> Task {
    let t = wait_until(m, id, "completion", |t| {
        matches!(t.status, Status::Completed | Status::Failed)
    })
    .await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    t
}

fn req(url: String) -> DownloadRequest {
    DownloadRequest { url, ..Default::default() }
}

fn assert_file(t: &Task, expected: &[u8]) {
    let got = std::fs::read(t.final_path().unwrap()).unwrap();
    assert_eq!(got.len(), expected.len());
    assert!(got == expected, "downloaded bytes differ");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn splits_into_many_connections() {
    let server = start(
        4 << 20,
        Behaviour { delay_ms: 3, breaks_if_range: true, ..Default::default() },
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let id = m.add(req(format!("{}/file.zip", server.base)), true).unwrap();
    let most = std::cell::Cell::new(0);
    let t = wait_until(&m, id, "completion", |t| {
        most.set(most.get().max(t.segments.active_count()));
        t.status == Status::Completed
    })
    .await;
    assert_file(&t, &server.body);
    assert!(server.peak.load(SeqCst) >= 4, "peak {}", server.peak.load(SeqCst));
    assert!((4..=6).contains(&most.get()), "client used {} connections", most.get());
    assert!(t.segments.segs.len() >= 6);
    assert!(!t.part_path().unwrap().exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn survives_dropped_connections() {
    let server = start(3 << 20, Behaviour { drops: AtomicUsize::new(7), ..Default::default() }).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    assert_file(&finished(&m, id).await, &server.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn respects_a_server_connection_limit() {
    let server = start(
        2 << 20,
        Behaviour { connection_cap: 2, delay_ms: 2, ..Default::default() },
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    assert_file(&finished(&m, id).await, &server.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_then_resume_after_restarting_the_app() {
    let server = start(3 << 20, Behaviour { delay_ms: 8, ..Default::default() }).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    wait_until(&m, id, "some data", |t| t.downloaded() > 600_000).await;
    m.pause(id);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let saved = task(&m, id);
    assert!(saved.downloaded() < 3 << 20);
    drop(m);

    let m = Manager::open(dir.path().join("data"), Handle::current()).unwrap();
    assert_eq!(task(&m, id).downloaded(), saved.downloaded());
    server.ranges.lock().unwrap().clear();
    m.resume(id);
    assert_file(&finished(&m, id).await, &server.body);
    let ranges = server.ranges.lock().unwrap();
    assert!(ranges.iter().all(|&r| r > 0), "resume must not start over: {ranges:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovers_from_a_crash_using_the_last_checkpoint() {
    let server = start(3 << 20, Behaviour { delay_ms: 200, ..Default::default() }).await;
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let crashed = Manager::open(dir.path().join("data"), rt.handle().clone()).unwrap();
    let settings = manager(dir.path(), |_| {}).settings();
    crashed.update_settings(settings).unwrap();
    let id = crashed.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    let state = dir.path().join("data/tasks/1.json");
    let start_time = Instant::now();
    loop {
        crashed.tick_at(0, 0);
        let saved: Option<Task> = std::fs::read(&state).ok().and_then(|d| serde_json::from_slice(&d).ok());
        if saved.is_some_and(|t| t.downloaded() > 200_000) {
            break;
        }
        assert!(start_time.elapsed() < Duration::from_secs(30), "no checkpoint written");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Kill everything without any orderly shutdown.
    rt.shutdown_background();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(crashed);

    let m = Manager::open(dir.path().join("data"), Handle::current()).unwrap();
    assert_eq!(task(&m, id).status, Status::Queued);
    assert!(task(&m, id).downloaded() > 0);
    assert_file(&finished(&m, id).await, &server.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuses_to_mix_two_versions_of_a_file() {
    let server = start(2 << 20, Behaviour { delay_ms: 10, ..Default::default() }).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    wait_until(&m, id, "some data", |t| t.downloaded() > 300_000).await;
    m.pause(id);
    *server.etag.lock().unwrap() = "\"v2\"".into();
    m.resume(id);
    let t = wait_until(&m, id, "failure", |t| t.status == Status::Failed).await;
    assert!(t.error.unwrap().contains("changed"));
    m.restart(id);
    assert_file(&finished(&m, id).await, &server.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_connection_when_ranges_are_unsupported() {
    for unknown_length in [false, true] {
        let server = start(
            1 << 20,
            Behaviour { ignore_range: true, unknown_length, ..Default::default() },
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let m = manager(dir.path(), |_| {});
        let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
        let t = finished(&m, id).await;
        assert_file(&t, &server.body);
        assert_eq!(t.resumable, Some(false));
        assert_eq!(t.total, Some(1 << 20));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handles_empty_files_and_never_overwrites() {
    let server = start(0, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    std::fs::write(dir.path().join("out/a.bin"), b"keep me").unwrap();
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    let t = finished(&m, id).await;
    assert_eq!(t.file_name, "a (1).bin");
    assert_eq!(std::fs::read(dir.path().join("out/a.bin")).unwrap(), b"keep me");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cookies_never_follow_a_redirect_to_another_host() {
    let target = start(1 << 20, Behaviour::default()).await;
    let origin = start(10, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let mut r = req(format!("{}/redirect?to={}/a.bin", origin.base, target.base));
    r.cookie = Some("session=secret".into());
    let id = m.add(r, true).unwrap();
    assert_file(&finished(&m, id).await, &target.body);
    assert!(origin.cookies.lock().unwrap().iter().any(|c| c.1.is_some()));
    let leaked = target.cookies.lock().unwrap();
    assert!(leaked.len() > 1);
    assert!(leaked.iter().all(|c| c.1.is_none()), "cookie leaked: {leaked:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticates_checks_hashes_and_reports_errors() {
    let server = start(200_000, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let mut r = req(format!("{}/basic", server.base));
    r.username = Some("u".into());
    r.password = Some("p".into());
    let id = m.add(r, true).unwrap();
    assert_file(&finished(&m, id).await, &server.body);

    let mut bad = req(format!("{}/a.bin", server.base));
    bad.checksum = Some("sha256:00".into());
    let id = m.add(bad, true).unwrap();
    let t = wait_until(&m, id, "checksum failure", |t| t.status == Status::Failed).await;
    assert!(t.error.unwrap().contains("checksum mismatch"));

    let id = m.add(req(format!("{}/missing", server.base)), true).unwrap();
    let t = wait_until(&m, id, "404", |t| t.status == Status::Failed).await;
    assert!(t.error.unwrap().contains("404"));
    assert!(m.add(req("file:///etc/passwd".into()), true).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn speed_limit_slows_the_transfer() {
    let server = start(1 << 20, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| s.speed_limit = 512 * 1024);
    let begin = Instant::now();
    let id = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    assert_file(&finished(&m, id).await, &server.body);
    let secs = begin.elapsed().as_secs_f64();
    assert!((1.5..5.0).contains(&secs), "1 MiB at 512 KiB/s took {secs:.2}s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_limits_concurrency_and_obeys_the_schedule() {
    let server = start(400_000, Behaviour { delay_ms: 10, ..Default::default() }).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| {
        s.max_active = 1;
        s.schedule.enabled = true;
        s.schedule.start_min = 60;
        s.schedule.stop_min = 120;
    });
    let a = m.add(req(format!("{}/a.bin", server.base)), true).unwrap();
    let b = m.add(req(format!("{}/b.bin", server.base)), true).unwrap();
    m.tick_at(0, 30);
    assert_eq!(task(&m, a).status, Status::Queued, "window is closed");
    m.tick_at(0, 90);
    assert_eq!(task(&m, a).status, Status::Downloading);
    assert_eq!(task(&m, b).status, Status::Queued, "only one at a time");
    m.tick_at(0, 121);
    assert_eq!(task(&m, a).status, Status::Queued, "stopped when window closes");
    m.resume(b);
    assert_file(&finished(&m, b).await, &server.body); // forced past the schedule
    assert_eq!(task(&m, a).status, Status::Queued);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_an_encrypted_hls_stream_via_the_best_variant() {
    use aes::cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
    let server = start(1, Behaviour::default()).await;
    let key = [7u8; 16];
    let clear: Vec<Vec<u8>> = (0..12).map(|i| payload(20_000 + i * 321)).collect();
    let mut media = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:3\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n");
    {
        let mut routes = server.routes.lock().unwrap();
        for (i, data) in clear.iter().enumerate() {
            let iv = (3 + i as u128).to_be_bytes();
            let enc = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
                .encrypt_padded_vec::<Pkcs7>(data);
            routes.push((format!("/v/s{i}.ts"), enc));
            media += &format!("#EXTINF:4,\ns{i}.ts\n");
        }
        media += "#EXT-X-ENDLIST\n";
        routes.push(("/v/key".into(), key.to_vec()));
        routes.push(("/v/high.m3u8".into(), media.into_bytes()));
        routes.push((
            "/v/master.m3u8".into(),
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900\nhigh.m3u8\n".to_vec(),
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |_| {});
    let mut r = req(format!("{}/v/master.m3u8", server.base));
    r.filename = Some("My show".into());
    let id = m.add(r, true).unwrap();
    let t = finished(&m, id).await;
    assert_eq!(t.file_name, "My show.ts");
    assert_file(&t, &clear.concat());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn grabs_a_page_then_downloads_everything_in_bulk() {
    let server = start(300_000, Behaviour::default()).await;
    let page: String = (1..=25).map(|i| format!("<a href=\"/f/{i}.zip\">{i}</a>")).collect();
    server.routes.lock().unwrap().push(("/list".into(), page.into_bytes()));
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| s.max_active = 5);
    let found = fluxdm_core::links::grab_page(&m.settings(), &format!("{}/list", server.base))
        .await
        .unwrap();
    assert_eq!(found.len(), 25);
    let ids: Vec<u64> = found.iter().map(|l| m.add(req(l.url.clone()), true).unwrap()).collect();
    for id in ids {
        assert_file(&finished(&m, id).await, &server.body);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn links_ticked_in_the_picker_are_not_confirmed_twice() {
    use fluxdm_core::ipc::{self, BrowserItem, Command};
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| s.confirm_browser = true);
    ipc::serve(m.clone(), Arc::new(|| {})).await.unwrap();
    for confirmed in [false, true] {
        let data = m.data_dir().to_path_buf();
        let items = vec![BrowserItem { url: "http://127.0.0.1:9/a.zip".into(), ..Default::default() }];
        let reply = tokio::task::spawn_blocking(move || ipc::request(&data, Command::Add { items, confirmed }))
            .await
            .unwrap()
            .unwrap();
        assert!(reply.ok);
    }
    assert_eq!(m.pending().unwrap().1, 1, "the unconfirmed one waits");
    assert_eq!(m.list().len(), 1, "the confirmed one was added");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previews_name_size_and_folder_that_the_download_then_uses() {
    use fluxdm_core::preview;
    let server = start(300_000, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| s.category_folders = true);
    let url = format!("{}/pack.zip", server.base);
    let info = preview::inspect(&m.settings(), &req(url.clone())).await.unwrap();
    assert_eq!((info.size, info.resumable, info.is_web_page), (Some(300_000), Some(true), false));
    assert!(m.list().is_empty(), "looking must not add a download");
    let folder = preview::folder_for(&m.settings(), &info.name);

    let id = m.add(req(url), true).unwrap();
    let task = finished(&m, id).await;
    assert_eq!((task.file_name.as_str(), task.dir.as_deref()), (info.name.as_str(), Some(folder.as_path())));
    assert_eq!(preview::free_name(&folder, &info.name), "pack (1).zip");

    let gone = preview::inspect(&m.settings(), &req("http://127.0.0.1:9/x.zip".into())).await;
    assert!(gone.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reports_each_finished_or_failed_download_exactly_once() {
    let server = start(200_000, Behaviour::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), |s| {
        s.max_active = 8;
        s.max_retries = 0;
    });
    let good: Vec<u64> = (0..12)
        .map(|i| m.add(req(format!("{}/f{i}.zip", server.base)), true).unwrap())
        .collect();
    let bad = m.add(req("http://127.0.0.1:9/nothing.zip".into()), true).unwrap();
    let waiting = m.add(req(format!("{}/later.zip", server.base)), false).unwrap();
    let mut events = Vec::new();
    for id in good.iter().chain([&bad]) {
        for _ in 0..1500 {
            m.tick_at(0, 0);
            events.extend(m.take_events());
            if events.iter().any(|e: &fluxdm_core::Event| e.id == *id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    for _ in 0..20 {
        m.tick_at(0, 0);
        events.extend(m.take_events());
    }
    let mut ids: Vec<u64> = events.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    let mut expected = good.clone();
    expected.push(bad);
    assert_eq!(ids, expected, "one event each, none for the paused download {waiting}");
    for e in &events {
        assert_eq!(e.failed(), e.id == bad, "{e:?}");
        assert_eq!(e.path.as_ref().is_some_and(|p| p.exists()), e.id != bad);
    }
}
