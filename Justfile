set dotenv-load

default:
	@just --list --unsorted --color=always

DH_USER := "mtinside"
GH_USER := "mt-inside"
DH_REPO := "docker.io/" + DH_USER + "/gertrude"
GH_REPO := "ghcr.io/" + GH_USER + "/gertrude"
TAG := `git describe --tags --always --abbrev`
TAGD := `git describe --tags --always --abbrev --dirty --broken`
CGR_ARCHS := "aarch64" # amd64,x86,armv7 - will fail cause no wolfi packages for these archs
MELANGE := "melange"
APKO    := "apko"

build-env:
	docker run -ti --rm -v ${PWD}:/work rust

tools-install:
	#!/bin/bash
	rustup toolchain install nightly
	rustup component add rustfmt --toolchain nightly
	rustup component add clippy
	rustup component add llvm-tools-preview
	cargo install grcov
	cargo install cargo-udeps
	cargo install cargo-minimal-versions
	cargo install cargo-hack
	if command -v apt > /dev/null 2>&1; then
		apt update && apt install -y protobuf-compiler
	elif command -v brew > /dev/null 2>&1; then
		brew install protobuf
	else
		echo "Unsupported package manager"
	fi

# non-blocking checks and advisories
analyze:
	cargo deny --all-features check

lint:
	cargo +nightly fmt --all --check
	cargo check
	cargo clippy --frozen --workspace --all-targets --all-features -- -D warnings
	cargo doc --frozen --no-deps # will fail if there's link issues etc
	cargo +nightly udeps --all-targets
	# ideally wouldn't use --direct
	cargo minimal-versions check --direct --workspace # check it builds with the min versions of all the deps (from their semver ranges)

test: lint
	cargo test --all

test-with-coverage: lint
	#!/bin/bash
	export RUSTFLAGS="-Cinstrument-coverage"
	export LLVM_PROFILE_FILE="gertrude-%p-%m.profraw"
	cargo test --all
	# Convert the profraw files into lcov
	mkdir -p target/debug/coverage
	grcov . -s . --binary-path target/debug/ -t lcov --branch --ignore-not-existing --keep-only 'src/*' -o target/debug/coverage/
	rm -f *profraw

coverage-view: test-with-coverage
	mkdir -p target/debug/coverage
	grcov . -s . --binary-path target/debug/ -t html --branch --ignore-not-existing --keep-only 'src/*' -o target/debug/coverage/
	open target/debug/coverage/html/index.html

fmt:
	cargo +nightly fmt --all

build: test
	cargo build

build-ci:
	cargo build --release

run-freenode: test
	RUST_BACKTRACE=1 cargo run -- -s chat.freenode.net -c '#test' --persist-path karma.binpb --plugin-dir ${PWD}/../gertrude-regex/target/wasm32-unknown-unknown/debug/

package: test
	rm -rf ./packages/
	{{MELANGE}} bump melange.yaml {{TAGD}}
	{{MELANGE}} keygen
	{{MELANGE}} build --arch {{CGR_ARCHS}} --signing-key melange.rsa melange.yaml

run *ARGS: test
	cargo run -- {{ARGS}}

image-local:
	{{APKO}} build --keyring-append melange.rsa.pub --arch {{CGR_ARCHS}} apko.yaml {{GH_REPO}}:{{TAG}} gertrude.tar
	docker load < gertrude.tar
image-publish:
	{{APKO}} login docker.io -u {{DH_USER}} --password "${DH_TOKEN}"
	{{APKO}} login ghcr.io   -u {{GH_USER}} --password "${GH_TOKEN}"
	{{APKO}} publish --keyring-append melange.rsa.pub --arch {{CGR_ARCHS}} apko.yaml {{GH_REPO}}:{{TAG}} {{DH_REPO}}:{{TAG}}
