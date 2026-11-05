# Writes the JSON Schema for deploy.yaml, and the website's copy of it.
schema:
    cargo run -q -p shipdeck -- --schema > deploy.schema.json
    cp deploy.schema.json docs/website/public/deploy.schema.json

# Writes the JSON Schema of the protocol messages, from their Rust types.
protocol:
    cargo run -q -p shipdeck-protocol --bin schema > protocol.schema.json

deploy-doc: schema
    cd ./docs/website/ && pnpm run deploy

dev-doc:
    cd ./docs/website && pnpm run dev
