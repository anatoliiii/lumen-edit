//! Remote helper started by the editor over SSH. It speaks the framed
//! protocol from `lumen_shared::proto` on stdin/stdout.

use std::{
    io::{self, BufReader, BufWriter},
    sync::{Arc, Mutex},
    thread,
};

use lumen_shared::{
    proto::{self, Message, Op, Request},
    watch,
};

type Output = Arc<Mutex<BufWriter<io::Stdout>>>;

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("lumen-agent {} {}", lumen_shared::VERSION, std::env::consts::ARCH);
        return;
    }

    let output: Output = Arc::new(Mutex::new(BufWriter::new(io::stdout())));
    let mut input = BufReader::new(io::stdin());
    let mut watcher: Option<watch::TreeWatcher> = None;

    // Ends when the editor goes away.
    while let Ok((request, payload)) = proto::read_frame::<_, Request>(&mut input) {
        if let Op::Watch { root } = &request.op {
            // Folders are added in the background, so requests are not held
            // up by a big tree.
            let events = output.clone();
            let result = watch::watch_tree(root, move |paths| send(&events, Message::FsChanged { paths }, &[])).map(|new_watcher| {
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
