use crate::output;

pub fn run(json: bool) {
    let version = env!("CARGO_PKG_VERSION");
    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "version": version,
            "name": "MyDownloader"
        }));
    } else {
        output::print_human(&format!("MyDownloader {version}"));
    }
}
