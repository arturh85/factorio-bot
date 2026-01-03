start:
    cd app; yarn run tauri:serve

repl *ARGS:
    cargo repl {{ARGS}}

factorio *ARGS:
    cargo run --release --no-default-features --features cli,repl -- start -v {{ARGS}}

test:
    cargo fmt --all
    cargo fmt --all -- --check
    cargo clippy --fix --workspace --tests --allow-dirty
    cargo test --workspace --quiet
    cargo build --release
