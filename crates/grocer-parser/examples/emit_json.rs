fn main() {
    let text = include_str!("../tests/fixtures/costco_652_redacted.txt");
    println!("{}", serde_json::to_string(&grocer_parser::parse(text)).unwrap());
}
