//! IPC round trips against an in-process headless service.

use std::sync::Arc;
use std::time::Duration;

use aw_manual_core::engine::ErrorCode;
use aw_manual_core::ipc::{self, ClientError, Endpoint};
use aw_manual_core::service::{Headless, Service};
use serde_json::{json, Value};

fn tempdir(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("aw-manual-ipc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn round_trips() {
    let dir = tempdir("rt");
    let cfg = dir.join("config");
    std::fs::create_dir_all(cfg.join("favorites.d")).unwrap();
    std::fs::write(
        cfg.join("config.yaml"),
        "server:\n  url: http://127.0.0.1:9\n  bucket_id: test_{hostname}\n",
    )
    .unwrap();
    std::fs::write(
        cfg.join("favorites.d/10-a.yaml"),
        "favorites:\n  ACME:\n    _color: \"#E4572E\"\n    Website: [Development]\n  Break:\n    _kind: status\n",
    )
    .unwrap();
    std::fs::write(
        cfg.join("favorites.d/20-bad.yaml"),
        "favorites:\n  X:\n    _bad: 1\n",
    )
    .unwrap();

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let svc = Service::start(
        cfg.clone(),
        &dir.join("db.sqlite"),
        Headless {
            shutdown: shutdown.clone(),
        },
    )
    .unwrap();
    #[cfg(unix)]
    let ep = Endpoint::Path(dir.join("run/daemon.sock"));
    #[cfg(windows)]
    let ep = Endpoint::Pipe(format!("aw-manual-test-{}", std::process::id()));
    let listener = ipc::bind(&ep).unwrap();
    tokio::spawn(ipc::serve(listener, svc.clone()));

    // A second daemon must not take over the endpoint.
    #[cfg(unix)]
    assert_eq!(
        ipc::bind(&ep).unwrap_err().kind(),
        std::io::ErrorKind::AddrInUse
    );

    let call = |method: &'static str, params: Value| {
        let ep = ep.clone();
        blocking(move || ipc::call(&ep, method, params, Duration::from_secs(5)))
    };

    let pong = call("ping", Value::Null).await.unwrap();
    assert_eq!(pong["protocol"], 1);

    let list = call("list", Value::Null).await.unwrap();
    assert_eq!(list["favorites"][0]["id"], "acme");
    assert_eq!(
        list["favorites"][0]["children"][0]["children"][0]["color"]["source"],
        "inherited"
    );
    assert_eq!(list["files"][1]["status"], "invalid");
    assert_eq!(list["files"][1]["errors"][0]["line"], 3);

    // Errors carry stable codes.
    let err = call(
        "start",
        json!({"target": {"favorite": "nope"}, "source": "cli"}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ClientError::Remote(ref e) if e.code == ErrorCode::NotFound),
        "{err}"
    );
    let err = call(
        "start",
        json!({"target": {"favorite": "acme"}, "source": "cli"}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ClientError::Remote(ref e) if e.code == ErrorCode::IsFolder),
        "{err}"
    );
    let err = call("bogus", Value::Null).await.unwrap_err();
    assert!(matches!(err, ClientError::Remote(ref e) if e.code == ErrorCode::BadRequest));
    let err = call("popover", Value::Null).await.unwrap_err();
    assert!(matches!(err, ClientError::Remote(ref e) if e.code == ErrorCode::Unavailable));

    // Watch: the first message is the current status, then pushes follow.
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    let ep2 = ep.clone();
    std::thread::spawn(move || {
        let mut c = ipc::Client::connect(&ep2).unwrap();
        let _ = c.watch(|v| tx.send(v).is_ok());
    });
    let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(first["running"].is_null());

    let r = call(
        "start",
        json!({"target": {"favorite": "acme/website/development"}, "note": "n", "source": "cli"}),
    )
    .await
    .unwrap();
    assert_eq!(r["change"], "started");
    assert_eq!(r["status"]["running"]["id"], "acme/website/development");
    assert_eq!(r["status"]["running"]["color"], "#E4572E");
    assert_eq!(
        r["status"]["server"]["bucket"],
        format!("test_{}", aw_manual_core::paths::hostname())
    );

    let pushed = loop {
        let v = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if !v["running"].is_null() {
            break v;
        }
    };
    assert_eq!(pushed["running"]["note"], "n");

    let r = call(
        "toggle",
        json!({"target": {"favorite": "acme/website/development"}, "source": "cli"}),
    )
    .await
    .unwrap();
    assert_eq!(r["change"], "stopped");
    let r = call("resume", json!({"source": "cli"})).await.unwrap();
    assert_eq!(r["change"], "started");
    let r = call("note", json!({"note": "m", "source": "cli"}))
        .await
        .unwrap();
    assert_eq!(r["change"], "note_changed");
    let r = call("stop", json!({"source": "cli"})).await.unwrap();
    assert_eq!(r["change"], "stopped");
    let err = call("stop", json!({"source": "cli"})).await.unwrap_err();
    assert!(matches!(err, ClientError::Remote(ref e) if e.code == ErrorCode::NotRunning));

    // Offline: entries are buffered.
    let st = call("status", Value::Null).await.unwrap();
    assert!(st["server"]["pending"].as_u64().unwrap() >= 1);

    // Reload picks up file changes.
    std::fs::write(
        cfg.join("favorites.d/20-bad.yaml"),
        "favorites:\n  Fixed:\n",
    )
    .unwrap();
    let r = call("reload", Value::Null).await.unwrap();
    assert_eq!(r["files"][1]["status"], "ok");
    assert_eq!(r["favorites"][2]["id"], "fixed");

    // Protocol version mismatch is reported.
    let ep3 = ep.clone();
    let raw = blocking(move || {
        use std::io::{BufRead, Write};
        let name = match &ep3 {
            #[cfg(unix)]
            Endpoint::Path(p) => {
                use interprocess::local_socket::prelude::*;
                p.clone()
                    .to_fs_name::<interprocess::local_socket::GenericFilePath>()
                    .unwrap()
            }
            #[allow(unreachable_patterns)]
            _ => return String::new(),
        };
        use interprocess::local_socket::prelude::*;
        let s = interprocess::local_socket::Stream::connect(name).unwrap();
        let mut r = std::io::BufReader::new(s);
        r.get_mut()
            .write_all(b"{\"v\":99,\"id\":7,\"method\":\"ping\"}\nnot json\n")
            .unwrap();
        let mut a = String::new();
        r.read_line(&mut a).unwrap();
        let mut b = String::new();
        r.read_line(&mut b).unwrap();
        a + &b
    })
    .await;
    #[cfg(unix)]
    {
        assert!(raw.contains("unsupported protocol version"), "{raw}");
        assert!(raw.contains("malformed request"), "{raw}");
    }
    let _ = raw;

    std::fs::remove_dir_all(&dir).unwrap();
}
