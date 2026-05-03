//! Generate the OpenAPI 3.x spec for the public API.
//!
//! Usage:
//!
//! ```text
//! cargo run --bin export_openapi               # write JSON to stdout
//! cargo run --bin export_openapi openapi.json  # write JSON to file
//! ```
//!
//! The file form writes deterministic LF line endings without a BOM,
//! independent of the calling shell — important so the committed file is
//! identical across Windows and Linux dev/CI environments.

fn main() {
    let api = chainsmith::api::openapi();
    let mut json = api
        .to_pretty_json()
        .expect("OpenAPI spec is generated from typed Rust structs and is always serializable");
    json.push('\n');

    match std::env::args().nth(1) {
        Some(path) => {
            std::fs::write(&path, json.as_bytes())
                .unwrap_or_else(|e| panic!("writing {path}: {e}"));
        }
        None => print!("{json}"),
    }
}
