tools-install:
	cargo install cargo-llvm-cov

lint:
	cargo clippy

test: lint
	cargo llvm-cov

run: test
	cargo run
