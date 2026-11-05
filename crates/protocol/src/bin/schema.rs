//! Prints the protocol's JSON Schema; `just protocol` writes it to the root.

fn main() {
    print!("{}", shipdeck_protocol::schema::text());
}
