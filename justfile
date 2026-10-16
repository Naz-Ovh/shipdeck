# Writes the JSON Schema for deploy.yaml, and the website's copy of it.
schema:
    cargo run -q -p shipdeck -- --schema > deploy.schema.json
    cp deploy.schema.json docs/website/public/deploy.schema.json

deploy-doc: schema
    cd ./docs/website/ && pnpm run deploy
