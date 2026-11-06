//! Prints the protocol's JSON Schema; `just protocol` writes it to
//! `docs/schema/`. An example, not a program: `cargo install` never puts it on
//! your PATH.

fn main() {
    print!("{}", shipdeck_protocol::schema::text());
}
