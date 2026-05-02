fn main() {
    let api = chainsmith::api::openapi();
    let json = api
        .to_pretty_json()
        .expect("OpenAPI spec is generated from typed Rust structs and is always serializable");
    println!("{json}");
}
