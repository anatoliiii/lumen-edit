//! Remote helper started by the editor over SSH. It speaks the framed
//! protocol from `codium_shared::proto` on stdin/stdout.

use std::{
    collections::BTreeSet,
    io::{self, BufReader, BufWriter},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use codium_shared::proto::{self, Message, Op, Request};
use notify::{RecursiveMode, Watcher};

type Output = Arc<Mutex<BufWriter<io::Stdout>>>;

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("codium-agent {} {}", codium_shared::VERSION, std::env::consts::ARCH);
        return;
    }

    let output: Output = Arc::new(Mutex::new(BufWriter::new(io::stdout())));
    let mut input = BufReader::new(io::stdin());
    let mut watcher: Option<notify::RecommendedWatcher> = None;

    // Ends when the editor goes away.
    while let Ok((request, payload)) = proto::read_frame::<_, Request>(&mut input) {
        if let Op::Watch { root } = &request.op {
            let result = start_watch(root, output.clone()).map(|new_watcher| {
                watcher = Some(new_watcher);
                proto::Reply::Ok
            });
            send(&output, Message::Response { id: request.id, result }, &[]);
            continue;
        }

        let output = output.clone();
        thread::spawn(move || {
            let (result, data) = match proto::execute(&request.op, &payload) {
                Ok((reply, data)) => (Ok(reply), data),
                Err(error) => (Err(error), Vec::new()),
            };
            send(&output, Message::Response { id: request.id, result }, &data);
        });
    }
    drop(watcher);
}

fn send(output: &Output, message: Message, payload: &[u8]) {
    if let Ok(mut out) = output.lock() {
        let _ = proto::write_frame(&mut *out, &message, payload);
    }
}

fn start_watch(root: &str, output: Output) -> Result<notify::RecommendedWatcher, String> {
    let (tx, rx) = mpsc::channel::<notify::Event>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            let _ = tx.send(event);
        }
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(std::path::Path::new(root), RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    // Batch bursts of events (builds, checkouts) into one message.
    thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut paths = BTreeSet::new();
            let mut add = |event: notify::Event| {
                for path in event.paths {
                    let path = path.to_string_lossy().into_owned();
                    if !path.contains("/.git/objects") && !path.contains("/target/") {
                        paths.insert(path);
                    }
                }
            };
            add(first);
            while let Ok(event) = rx.recv_timeout(Duration::from_millis(150)) {
                add(event);
            }
            if !paths.is_empty() {
                send(&output, Message::FsChanged { paths: paths.into_iter().collect() }, &[]);
            }
        }
    });
    Ok(watcher)
}
