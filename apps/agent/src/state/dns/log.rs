//! Best-effort point-in-time observations must never block DNS expiry or IO.

enum Output {
    Event(String),
    Warning(String),
}

fn emit(output: Output) {
    static OUTPUT: std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<Output>>> =
        std::sync::OnceLock::new();
    if let Some(sender) = OUTPUT.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::sync_channel::<Output>(32);
        std::thread::Builder::new()
            .name("meshway-dns-observation".into())
            .spawn(move || {
                use std::io::Write;
                for output in receive {
                    let result = match output {
                        Output::Event(json) => writeln!(std::io::stderr(), "{json}"),
                        Output::Warning(message) => writeln!(std::io::stderr(), "{message}"),
                    };
                    if result.is_err() {
                        break;
                    }
                }
            })
            .ok()
            .map(|_| send)
    }) {
        let _ = sender.try_send(output);
    }
}

pub(crate) fn event(json: String) {
    emit(Output::Event(json));
}
pub(crate) fn warning(message: String) {
    emit(Output::Warning(message));
}
