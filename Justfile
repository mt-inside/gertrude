set dotenv-load

default:
	@just --list --unsorted --color=always

DH_USER := "mtinside"
GH_USER := "mt-inside"
DH_REPO := "docker.io/" + DH_USER + "/http-log"
GH_REPO := "ghcr.io/" + GH_USER + "/http-log"
TAG := `git describe --tags --always --abbrev`
TAGD := `git describe --tags --always --abbrev --dirty --broken`
CGR_ARCHS := "aarch64" # amd64,x86,armv7 - will fail cause no wolfi packages for these archs
MELANGE := "melange"
APKO    := "apko"

tools-install:
	cargo install cargo-llvm-cov

lint:
	cargo clippy

test: lint
	cargo llvm-cov

build-ci:
	cargo build --release

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
