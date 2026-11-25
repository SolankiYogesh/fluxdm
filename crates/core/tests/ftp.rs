//! FTP download and resume against a minimal in-process server.

use fluxdm_core::{DownloadRequest, Manager, Settings, Status, Task};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

async fn control(
    stream: tokio::net::TcpStream,
    body: Arc<Vec<u8>>,
    fail_once: Arc<AtomicBool>,
    offsets: Arc<Mutex<Vec<usize>>>,
) {
    let (read, mut out) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let _ = out.write_all(b"220 hi\r\n").await;
    let mut data: Option<TcpListener> = None;
    let mut offset = 0;
    while let Ok(Some(line)) = lines.next_line().await {
        let (cmd, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let reply = match cmd {
            "USER" => "331 ok".to_string(),
            "PASS" => "230 ok".into(),
            "TYPE" => "200 ok".into(),
            "SIZE" => format!("213 {}", body.len()),
            "REST" => {
                offset = arg.parse().unwrap();
                "350 ok".into()
            }
            "PASV" => {
                let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let p = l.local_addr().unwrap().port();
                data = Some(l);
                format!("227 Entering Passive Mode (127,0,0,1,{},{})", p >> 8, p & 255)
            }
            "RETR" => {
                offsets.lock().unwrap().push(offset);
                let (mut sock, _) = data.take().unwrap().accept().await.unwrap();
                let _ = out.write_all(b"150 sending\r\n").await;
                let rest = &body[offset..];
                let cut = fail_once.swap(false, SeqCst);
                let _ = sock.write_all(if cut { &rest[..rest.len() / 2] } else { rest }).await;
                let _ = sock.shutdown().await;
                drop(sock);
                offset = 0;
                if cut { "426 aborted".into() } else { "226 done".into() }
            }
            "QUIT" => "221 bye".into(),
            _ => "502 no".into(),
        };
        if out.write_all(format!("{reply}\r\n").as_bytes()).await.is_err() {
            break;
        }
    }
}

async fn wait(m: &Manager, id: u64) -> Task {
    for _ in 0..1500 {
        m.tick_at(0, 0);
        let t = m.list().into_iter().find(|v| v.task.id == id).unwrap().task;
        if matches!(t.status, Status::Completed | Status::Failed) {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_over_ftp_and_resumes_after_a_broken_transfer() {
    let body: Arc<Vec<u8>> = Arc::new((0..900_000u32).map(|i| (i * 31 % 251) as u8).collect());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let offsets = Arc::new(Mutex::new(Vec::new()));
    let (b, o, fail) = (body.clone(), offsets.clone(), Arc::new(AtomicBool::new(true)));
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            tokio::spawn(control(s, b.clone(), fail.clone(), o.clone()));
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let m = Manager::open(dir.path().join("data"), Handle::current()).unwrap();
    m.update_settings(Settings {
        download_dir: dir.path().join("out"),
        category_folders: false,
        ..Settings::default()
    })
    .unwrap();
    let url = format!("ftp://127.0.0.1:{port}/pub/a.bin");
    let id = m.add(DownloadRequest { url, ..Default::default() }, true).unwrap();
    let t = wait(&m, id).await;
    assert_eq!(t.status, Status::Completed, "{:?}", t.error);
    assert!(std::fs::read(t.final_path().unwrap()).unwrap() == *body);
    let seen = offsets.lock().unwrap().clone();
    assert!(seen.len() >= 2 && seen[0] == 0 && seen[1] > 0, "{seen:?}");

    let url = format!("ftp://127.0.0.1:{port}/a%0D%0ADELE%20x");
    let id = m.add(DownloadRequest { url, ..Default::default() }, true).unwrap();
    let t = wait(&m, id).await;
    assert!(t.error.unwrap().contains("forbidden"));
}
